//! Links a page asked to open in the browser: asked about in a native dialog (the page cannot draw
//! over it), or opened at once for a site the user trusts from that server.

use tauri::{AppHandle, Manager};
use url::Url;

use super::main_window;
use crate::store::{RegistryStore, Store};

/// Opens a link a server's page asked for, once the user agrees in a native dialog (the page
/// cannot draw over it), or at once for a site they chose to trust from this server. One question
/// at a time; links asked for meanwhile are dropped.
pub fn ask_to_open(app: &AppHandle, entry_id: &str, server: &str, url: Url) {
    use shiver_core::links::{self, Answer, Decision};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};

    static ASKING: AtomicBool = AtomicBool::new(false);

    let decision = links::decide(
        &app.state::<Store>().registry().settings.trusted_links,
        entry_id,
        server,
        &url,
    );
    let (site, question, always) = match decision {
        Decision::Refuse => return,
        Decision::Open => return open_in_browser(app, &url),
        Decision::Ask {
            site,
            question,
            always,
        } => (site, question, always),
    };

    if ASKING.swap(true, Ordering::AcqRel) {
        return eprintln!("[shiver] {server} asked to open a link while another waited; dropped");
    }

    let mut dialog = app.dialog().message(question).title("Open link?").buttons(
        MessageDialogButtons::YesNoCancelCustom(links::OPEN.into(), always, links::CANCEL.into()),
    );

    if let Ok(window) = main_window(app) {
        dialog = dialog.parent(&window);
    }

    let (app, entry_id) = (app.clone(), entry_id.to_string());

    dialog.show_with_result(move |answer| {
        ASKING.store(false, Ordering::Release);

        let MessageDialogResult::Custom(choice) = answer else {
            return;
        };

        match Answer::from_choice(&choice, &site) {
            Answer::Cancel => return,
            Answer::Open => {}
            Answer::Always => {
                let _ = app.state::<Store>().update(|registry| {
                    links::trust(&mut registry.settings.trusted_links, &entry_id, site);

                    Ok(())
                });
            }
        }

        open_in_browser(&app, &url);
    });
}

fn open_in_browser(app: &AppHandle, url: &Url) {
    use tauri_plugin_opener::OpenerExt;

    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        eprintln!("[shiver] could not open {url} in the browser: {error}");
    }
}
