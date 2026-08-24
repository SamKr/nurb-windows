//! Which nurb and which adapter runtime the app actually runs: the dev
//! checkout (debug builds, through PATH uv and npx, as phases 1-5 did) or the
//! self-provisioned environment under app data that provision.rs installs.
//! Release builds only know the provisioned form; the checkout path is never
//! compiled into them.

use std::path::PathBuf;
use std::process::Command;

use crate::agents::AgentKind;

/// The Node runtime the provisioner installs. The adapters need >= 22; this
/// is the current LTS, pinned with its published tarball checksums so the
/// runtime download is verified.
pub const NODE_VERSION: &str = "v24.19.0";

#[derive(Clone)]
pub enum Launcher {
    Checkout { repo: PathBuf },
    Provisioned { paths: Paths },
}

impl Launcher {
    pub fn resolve(data: PathBuf) -> Self {
        #[cfg(debug_assertions)]
        if std::env::var_os("NURB_DESKTOP_PROVISIONED").is_none() {
            return Self::Checkout {
                repo: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
            };
        }
        Self::Provisioned {
            paths: Paths::new(data),
        }
    }

    pub fn paths(&self) -> Option<&Paths> {
        match self {
            Self::Checkout { .. } => None,
            Self::Provisioned { paths } => Some(paths),
        }
    }

    /// A command that runs the nurb CLI; callers append `dev`, `new`, etc.
    pub fn nurb(&self) -> Command {
        match self {
            Self::Checkout { repo } => {
                let mut command = Command::new("uv");
                command.args(["run", "--project"]).arg(repo).arg("nurb");
                command
            }
            Self::Provisioned { paths } => Command::new(paths.venv_bin("nurb")),
        }
    }

    /// Program and arguments that run an agent's ACP process. Native CLIs
    /// (Cursor, Grok) are the user's own install and spawn the same way in
    /// both modes. Provisioned adapters are spawned as `node <script>` rather
    /// than through the .bin shebang, which would resolve whatever node `env`
    /// finds on PATH.
    pub fn adapter(&self, kind: AgentKind) -> (String, Vec<String>) {
        if let Some((name, args)) = kind.native_command() {
            let program = kind
                .native_bin()
                .map(|bin| bin.to_string_lossy().into_owned())
                // Not found: spawn the bare name so the failure is an honest
                // "No such file", not a panic.
                .unwrap_or_else(|| name.into());
            return (program, args.iter().map(|a| a.to_string()).collect());
        }
        let pin = kind.adapter().expect("adapter-hosted");
        let acp_args = if kind == AgentKind::Gemini {
            vec!["--acp".into()]
        } else {
            Vec::new()
        };
        match self {
            Self::Checkout { .. } => {
                let mut args = vec!["-y".into(), pin.into()];
                args.extend(acp_args);
                // npx is a .cmd batch shim on Windows, and CreateProcess only
                // resolves .exe from a bare name.
                let npx = if cfg!(windows) { "npx.cmd" } else { "npx" };
                (npx.into(), args)
            }
            Self::Provisioned { paths } => (
                paths.node_bin().to_string_lossy().into_owned(),
                std::iter::once(paths.adapter_script(kind).to_string_lossy().into_owned())
                    .chain(acp_args)
                    .collect(),
            ),
        }
    }

    /// PATH for adapter processes. Agents run `nurb build` and friends while
    /// they work, and on an end-user machine the only nurb (and node) anywhere
    /// is the provisioned one, so their shells must see it. Checkout mode
    /// inherits the dev machine's PATH untouched.
    pub fn adapter_path(&self) -> Option<String> {
        match self {
            Self::Checkout { .. } => None,
            Self::Provisioned { paths } => {
                // Windows keeps console binaries in the venv's Scripts and at
                // the Node install's root; POSIX keeps both under bin/.
                let (venv_bin, node_bin) = if cfg!(windows) {
                    (paths.venv().join("Scripts"), paths.node_dir())
                } else {
                    (paths.venv().join("bin"), paths.node_dir().join("bin"))
                };
                let inherited = std::env::var_os("PATH").unwrap_or_else(|| {
                    if cfg!(windows) {
                        Default::default()
                    } else {
                        "/usr/bin:/bin".into()
                    }
                });
                let entries = [venv_bin, node_bin]
                    .into_iter()
                    .chain(std::env::split_paths(&inherited));
                std::env::join_paths(entries)
                    .ok()
                    .map(|joined| joined.to_string_lossy().into_owned())
            }
        }
    }

    /// The directory the engine writes while it works, granted to the agent
    /// sandbox: the provisioned app-data dir on user machines, the repo
    /// checkout in dev builds (where `uv run --project` and npx put venvs
    /// and caches).
    pub fn engine_root(&self) -> PathBuf {
        match self {
            Self::Checkout { repo } => repo.clone(),
            Self::Provisioned { paths } => paths.data().clone(),
        }
    }

    /// Whether this agent's ACP process can be spawned at all.
    pub fn adapter_available(&self, kind: AgentKind) -> bool {
        if kind.native_command().is_some() {
            return kind.native_bin().is_some();
        }
        match self {
            Self::Checkout { .. } => Command::new("npx")
                .arg("--version")
                .output()
                .map(|out| out.status.success())
                .unwrap_or(false),
            Self::Provisioned { paths } => {
                paths.node_bin().is_file() && paths.adapter_script(kind).is_file()
            }
        }
    }
}

/// Everything the provisioned environment owns lives under one app data dir,
/// including uv's Python installs and cache: ComfyUI Desktop let managed
/// Python land in uv's default location and got unrecoverable half-installs
/// out of it, so nothing here leaves this directory.
#[derive(Clone)]
pub struct Paths {
    data: PathBuf,
}

impl Paths {
    pub(crate) fn new(data: PathBuf) -> Self {
        Self { data }
    }

    pub fn data(&self) -> &PathBuf {
        &self.data
    }

    pub fn python_dir(&self) -> PathBuf {
        self.data.join("python")
    }

    pub fn uv_cache(&self) -> PathBuf {
        self.data.join("uv-cache")
    }

    pub fn venv(&self) -> PathBuf {
        self.data.join("env")
    }

    /// A console entry point inside the venv. Windows venvs put renamed-exe
    /// launchers in Scripts\; POSIX venvs put scripts in bin/.
    pub fn venv_bin(&self, name: &str) -> PathBuf {
        if cfg!(windows) {
            self.venv().join("Scripts").join(format!("{name}.exe"))
        } else {
            self.venv().join("bin").join(name)
        }
    }

    pub fn venv_python(&self) -> PathBuf {
        self.venv_bin("python")
    }

    pub fn node_dir(&self) -> PathBuf {
        self.data.join("node")
    }

    /// The Windows zip keeps node.exe at the archive root; the POSIX tarballs
    /// keep node under bin/.
    pub fn node_bin(&self) -> PathBuf {
        if cfg!(windows) {
            self.node_dir().join("node.exe")
        } else {
            self.node_dir().join("bin/node")
        }
    }

    /// npm as shipped inside the Node archive, invoked through its JS entry
    /// so nothing depends on PATH.
    pub fn npm_cli(&self) -> PathBuf {
        if cfg!(windows) {
            self.node_dir().join("node_modules/npm/bin/npm-cli.js")
        } else {
            self.node_dir().join("lib/node_modules/npm/bin/npm-cli.js")
        }
    }

    pub fn adapters(&self) -> PathBuf {
        self.data.join("adapters")
    }

    /// The script `node` runs to start an adapter. On POSIX, npm's .bin entry
    /// is a symlink to the package's JS entry, so it works as both. On
    /// Windows, .bin holds shell/cmd shims instead, so the entry is resolved
    /// from the installed package's own manifest; the bin table's relative
    /// path is exactly what npm built those shims from.
    pub fn adapter_script(&self, kind: AgentKind) -> PathBuf {
        let bin = kind.adapter_bin().expect("adapter-hosted");
        if !cfg!(windows) {
            return self.adapters().join("node_modules/.bin").join(bin);
        }
        let pin = kind.adapter().expect("adapter-hosted");
        let (package, _) = pin.rsplit_once('@').expect("adapter pins are name@version");
        let root = self.adapters().join("node_modules").join(package);
        js_entry(&root, bin).unwrap_or_else(|| root.join("dist/index.js"))
    }

    /// The Codex CLI npm installs as a dependency of the Codex adapter.
    /// codex-acp's ACP server falls back to this copy on its own, but its
    /// login path spawns a bare `codex` off PATH instead, so the app has to
    /// name it. See `CODEX_PATH` in agents.rs. On Windows the .bin shim and
    /// the JS launcher are both unspawnable as a plain executable, so this
    /// points at the real codex.exe inside the platform package npm installed
    /// as @openai/codex's optional dependency.
    pub fn codex_cli(&self) -> PathBuf {
        if !cfg!(windows) {
            return self.adapters().join("node_modules/.bin/codex");
        }
        let triple = match std::env::consts::ARCH {
            "aarch64" => "aarch64-pc-windows-msvc",
            _ => "x86_64-pc-windows-msvc",
        };
        let package = match std::env::consts::ARCH {
            "aarch64" => "codex-win32-arm64",
            _ => "codex-win32-x64",
        };
        self.adapters()
            .join("node_modules/@openai")
            .join(package)
            .join("vendor")
            .join(triple)
            .join("bin/codex.exe")
    }

    pub fn stamp(&self) -> PathBuf {
        self.data.join("provisioned.json")
    }
}

/// The JS file an installed npm package's bin table names, which is the file
/// its .bin shims run. `bin` is either a map (name -> path) or, for a package
/// with a single entry named after itself, a bare string. Only consulted on
/// Windows; POSIX spawns the .bin symlink directly.
fn js_entry(package_root: &std::path::Path, bin: &str) -> Option<PathBuf> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(package_root.join("package.json")).ok()?).ok()?;
    let relative = match &manifest["bin"] {
        serde_json::Value::String(path) => path.as_str(),
        table => table.get(bin)?.as_str()?,
    };
    Some(package_root.join(relative))
}

/// The bundled uv sidecar: Tauri strips the target-triple suffix and places
/// it next to the app executable (Contents/MacOS in a bundle, target/debug
/// during dev; tests run one level down in deps/).
pub fn uv_sidecar() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("no current exe: {e}"))?;
    let dir = exe.parent().ok_or("current exe has no parent")?;
    let dir = if dir.ends_with("deps") {
        dir.parent().unwrap()
    } else {
        dir
    };
    let uv = dir.join(if cfg!(windows) { "uv.exe" } else { "uv" });
    if uv.is_file() {
        Ok(uv)
    } else {
        Err(format!("bundled uv missing at {}", uv.display()))
    }
}
