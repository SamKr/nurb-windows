// The one platform signal the frontend needs without a round trip to Rust:
// wording that differs per OS (Recycle Bin vs Trash, Credential Manager vs
// Keychain). WebView2 always reports Windows in the user agent; WKWebView
// never does.
export const onWindows = navigator.userAgent.includes("Windows");
