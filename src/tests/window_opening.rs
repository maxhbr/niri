use std::fmt::{self, Write as _};

use insta::assert_snapshot;
use niri_config::Config;
use rayon::iter::{IntoParallelIterator, ParallelIterator};

use super::*;
use crate::utils::with_toplevel_role;

#[test]
fn simple_no_workspaces() {
    let mut f = Fixture::new();

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 0 × 0, bounds: 0 × 0, states: []"
    );

    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 100 × 688, bounds: 1248 × 688, states: []"
    );
}

#[test]
fn simple() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 936 × 1048, bounds: 1888 × 1048, states: []"
    );

    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @"size: 936 × 1048, bounds: 1888 × 1048, states: [Activated]"
    );
}

#[test]
fn dont_ack_initial_configure() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    // Don't ack the configure.
    window.commit();
    f.double_roundtrip(id);

    // FIXME: Technically this is a protocol violation but uh. Smithay currently doesn't check it,
    // and I'm not sure if it can be done generically in Smithay (because a compositor may not use
    // its rendering helpers). I might add a check in niri itself sometime; I'm just not sure if
    // there might be clients that this could break.
    let window = f.client(id).window(&surface);
    assert_snapshot!(
        window.format_recent_configures(),
        @r"
    size: 936 × 1048, bounds: 1888 × 1048, states: []
    size: 936 × 1048, bounds: 1888 × 1048, states: [Activated]
    "
    );
}

#[derive(Clone, Copy)]
enum WantFullscreen {
    No,
    UnsetBeforeInitial,
    BeforeInitial(Option<&'static str>),
    UnsetAfterInitial,
    AfterInitial(Option<&'static str>),
}

impl fmt::Display for WantFullscreen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WantFullscreen::No => write!(f, "U")?,
            WantFullscreen::UnsetBeforeInitial => write!(f, "BU")?,
            WantFullscreen::UnsetAfterInitial => write!(f, "AA")?,
            WantFullscreen::BeforeInitial(m) => write!(f, "B{}", m.unwrap_or("N"))?,
            WantFullscreen::AfterInitial(m) => write!(f, "A{}", m.unwrap_or("N"))?,
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum SetParent {
    BeforeInitial(&'static str),
    AfterInitial(&'static str),
}

impl fmt::Display for SetParent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetParent::BeforeInitial(m) => write!(f, "B{m}")?,
            SetParent::AfterInitial(m) => write!(f, "A{m}")?,
        }
        Ok(())
    }
}

#[test]
fn target_output_and_workspaces() {
    // Here we test a massive powerset of settings that can affect where a window opens:
    //
    // * open-on-workspace
    // * open-on-output
    // * has parent (windows will open next to their parent)
    // * want fullscreen (windows can request the target fullscreen output)
    // * open-fullscreen (can deny the fullscreen request)

    let open_on_workspace = [None, Some("1"), Some("2")];
    let open_on_output = [None, Some("1"), Some("2")];
    let open_fullscreen = [None, Some("false"), Some("true")];
    let want_fullscreen = [
        WantFullscreen::No,
        WantFullscreen::UnsetBeforeInitial, // GTK 4
        WantFullscreen::BeforeInitial(None),
        WantFullscreen::BeforeInitial(Some("1")),
        WantFullscreen::BeforeInitial(Some("2")),
        WantFullscreen::UnsetAfterInitial,
        // mpv, osu!
        WantFullscreen::AfterInitial(None),
        WantFullscreen::AfterInitial(Some("1")),
        WantFullscreen::AfterInitial(Some("2")),
    ];
    let set_parent = [
        None,
        Some(SetParent::BeforeInitial("1")),
        Some(SetParent::BeforeInitial("2")),
        Some(SetParent::AfterInitial("1")),
        Some(SetParent::AfterInitial("2")),
    ];

    let mut powerset = Vec::new();
    for ws in open_on_workspace {
        for out in open_on_output {
            for fs in open_fullscreen {
                for wfs in want_fullscreen {
                    for sp in set_parent {
                        powerset.push((ws, out, fs, wfs, sp));
                    }
                }
            }
        }
    }

    powerset.into_par_iter().for_each(|(ws, out, fs, wfs, sp)| {
        check_target_output_and_workspace(ws, out, fs, wfs, sp);
    });
}

fn check_target_output_and_workspace(
    open_on_workspace: Option<&str>,
    open_on_output: Option<&str>,
    open_fullscreen: Option<&str>,
    want_fullscreen: WantFullscreen,
    set_parent: Option<SetParent>,
) {
    let mut snapshot_desc = Vec::new();
    let mut snapshot_suffix = Vec::new();

    let mut config = String::from(
        r##"
workspace "ws-1" {
    open-on-output "headless-1"
}

workspace "ws-2" {
    open-on-output "headless-2"
}

window-rule {
    exclude title="parent"

"##,
    );

    if let Some(x) = open_on_workspace {
        writeln!(config, "    open-on-workspace \"ws-{x}\"").unwrap();
        snapshot_suffix.push(format!("ws{x}"));
    }

    if let Some(x) = open_on_output {
        writeln!(config, "    open-on-output \"headless-{x}\"").unwrap();
        snapshot_suffix.push(format!("out{x}"));
    }

    if let Some(x) = open_fullscreen {
        writeln!(config, "    open-fullscreen {x}").unwrap();

        let x = if x == "true" { "T" } else { "F" };
        snapshot_suffix.push(format!("fs{x}"));
    }
    config.push('}');

    match &want_fullscreen {
        WantFullscreen::No => (),
        x => {
            snapshot_desc.push(format!("want fullscreen: {x}"));
            snapshot_suffix.push(format!("wfs{x}"));
        }
    }

    if let Some(set_parent) = set_parent {
        let mon = match set_parent {
            SetParent::BeforeInitial(mon) => mon,
            SetParent::AfterInitial(mon) => mon,
        };
        write!(
            config,
            "

window-rule {{
    match title=\"parent\"
    open-on-output \"headless-{mon}\"
}}"
        )
        .unwrap();

        snapshot_desc.push(format!("set parent: {set_parent}"));
        snapshot_suffix.push(format!("sp{set_parent}"));
    }

    snapshot_desc.push(format!("config:{config}"));

    let config = Config::parse("config.kdl", &config).unwrap();

    let mut f = Fixture::with_config(config);
    f.add_output(1, (1280, 720));
    f.add_output(2, (1920, 1080));

    let id = f.add_client();

    // To get output names.
    f.roundtrip(id);

    let mut parent = None;
    if set_parent.is_some() {
        let window = f.client(id).create_window();
        let surface = window.surface.clone();
        parent = Some(window.xdg_toplevel.clone());
        window.set_title("parent");
        window.commit();
        f.roundtrip(id);

        let window = f.client(id).window(&surface);
        window.attach_new_buffer();
        window.ack_last_and_commit();
        f.roundtrip(id);
    }

    let client = f.client(id);
    let window = client.create_window();
    let surface = window.surface.clone();

    if let Some(SetParent::BeforeInitial(_)) = set_parent {
        client.window(&surface).set_parent(parent.as_ref());
    }

    if let WantFullscreen::UnsetBeforeInitial = want_fullscreen {
        client.window(&surface).unset_fullscreen();
    } else if let WantFullscreen::BeforeInitial(mon) = want_fullscreen {
        let output = mon.map(|mon| client.output(&format!("headless-{mon}")));
        client.window(&surface).set_fullscreen(output.as_ref());
    }

    client.window(&surface).commit();
    f.roundtrip(id);

    let client = f.client(id);
    let initial = client.window(&surface).format_recent_configures();

    if let Some(SetParent::AfterInitial(_)) = set_parent {
        client.window(&surface).set_parent(parent.as_ref());
    }

    if let WantFullscreen::UnsetAfterInitial = want_fullscreen {
        client.window(&surface).unset_fullscreen();
    } else if let WantFullscreen::AfterInitial(mon) = want_fullscreen {
        let output = mon.map(|mon| client.output(&format!("headless-{mon}")));
        client.window(&surface).set_fullscreen(output.as_ref());
    }

    let window = client.window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    let niri = f.niri();
    let (mon, ws_idx, ws) = niri
        .layout
        .workspaces()
        .find(|(_, _, ws)| {
            ws.windows().any(|win| {
                with_toplevel_role(win.toplevel(), |role| {
                    role.title.as_deref() != Some("parent")
                })
            })
        })
        .unwrap();
    let mon = mon.unwrap().output_name().clone();
    let ws = ws.name().cloned().unwrap_or(String::from("unnamed"));

    let window = f.client(id).window(&surface);
    let snapshot = format!(
        "\
final monitor: {mon}
final workspace: {ws_idx} ({ws})

initial configure:
{initial}

post-map configures:
{}",
        window.format_recent_configures()
    );

    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_suffix(snapshot_suffix.join("-"));
    settings.set_description(snapshot_desc.join("\n"));
    let _guard = settings.bind_to_scope();
    assert_snapshot!(snapshot);
}
