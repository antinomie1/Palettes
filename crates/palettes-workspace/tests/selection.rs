//! Each output keeps the window last focused on its shown workspace.

use palettes_workspace::{Desktop, Selection, Window, Workspace};

fn workspace(id: u64, output: &str, active: bool) -> Workspace {
    Workspace {
        id,
        name: id.to_string(),
        index: 1,
        output: output.into(),
        active,
        focused: false,
        windows: 0,
    }
}

fn window(id: u64, workspace: u64, focused: bool) -> Window {
    Window {
        id,
        title: format!("w{id}"),
        app_id: String::new(),
        icon: String::new(),
        workspace,
        focused,
        floating: false,
        fullscreen: false,
    }
}

fn desktop(windows: Vec<Window>) -> Desktop {
    Desktop {
        workspaces: vec![workspace(1, "A", true), workspace(2, "B", true)],
        windows,
        ..Desktop::default()
    }
}

#[test]
fn outputs_keep_their_last_focused_window() {
    let mut selection = Selection::default();
    let selected = |s: &Selection, d: &Desktop, output| s.on(d, output).map(|w| w.id);

    // Nothing focused yet: each output's first window.
    let start = desktop(vec![window(10, 1, false), window(11, 1, false), window(20, 2, false)]);
    selection.update(&start);
    assert_eq!(selected(&selection, &start, "A"), Some(10));
    assert_eq!(selected(&selection, &start, "B"), Some(20));

    // Focus 11 on A, then move to B: A still shows 11 as selected.
    let on_a = desktop(vec![window(10, 1, false), window(11, 1, true), window(20, 2, false)]);
    selection.update(&on_a);
    let on_b = desktop(vec![window(10, 1, false), window(11, 1, false), window(20, 2, true)]);
    selection.update(&on_b);
    assert_eq!(selected(&selection, &on_b, "A"), Some(11));
    assert_eq!(selected(&selection, &on_b, "B"), Some(20));

    // The remembered window closes: back to the first one left.
    let closed = desktop(vec![window(10, 1, false), window(20, 2, true)]);
    selection.update(&closed);
    assert_eq!(selected(&selection, &closed, "A"), Some(10));
    assert_eq!(selected(&selection, &closed, "C"), None);
}

#[test]
fn without_workspaces_the_focused_window_is_selected() {
    let mut selection = Selection::default();
    let d = Desktop {
        windows: vec![window(1, 0, false), window(2, 0, true)],
        ..Desktop::default()
    };
    selection.update(&d);
    assert_eq!(selection.on(&d, "A").map(|w| w.id), Some(2));
}
