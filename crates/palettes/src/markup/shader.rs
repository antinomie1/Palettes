//! WGSL files for `Shader`, resolved like every asset and compiled again
//! when they change.

use super::follow::Followers;
use aegle_widgets::ShaderView;
use std::path::Path;

thread_local! {
    static SHOWN: Followers<ShaderView> = const { Followers::new() };
}

/// Shows the program at `source`, relative to the asset roots; empty clears
/// it. A failure reaches the element's `failed` event and the log.
pub(crate) fn show(view: &ShaderView, source: &str) {
    if source.is_empty() {
        SHOWN.with(|shown| shown.forget(view));
        view.clear();
        return;
    }
    let path = super::roots()
        .resolve(source)
        .unwrap_or_else(|| source.into());
    let path = SHOWN.with(|shown| shown.follow(view, &path));
    load(view, &path);
}

/// Compiles `path` again for every view showing it.
pub(crate) fn changed(path: &Path) {
    for view in SHOWN.with(|shown| shown.of(path)) {
        load(&view, path);
    }
}

fn load(view: &ShaderView, path: &Path) {
    if let Err(e) = view.load(path) {
        eprintln!("palettes: shader {e}");
    }
}
