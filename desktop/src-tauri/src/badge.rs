//! The mark on Shiver's taskbar icon when something is waiting.
//!
//! Deliberately a dot rather than a number. The bell already says how many and which server they
//! came from; the taskbar only has to answer "is there anything?", from across the room and at
//! sixteen pixels. A count there would be a second, smaller copy of the inbox — and one that goes
//! stale the moment the window is not focused.
//!
//! Drawn here rather than shipped as a file. It is a circle in one colour, the size is decided by
//! the platform, and a generated one cannot fall out of step with the rail's badge the way a
//! checked-in `.png` would.

use tauri::{AppHandle, Manager};

use crate::{feed::Feed, webviews};

/// Side of the overlay icon. Windows draws it into the corner of the taskbar button and scales to
/// suit the display, so this only has to be large enough not to look soft.
const SIZE: u32 = 32;

/// `--shiver-danger`, the same colour the rail's unread badge uses.
const DOT: [u8; 3] = [0xff, 0x64, 0x67];

/// Shiver's own background, used as a rim. Without it the dot sits on a warm icon with almost no edge.
const RIM: [u8; 3] = [0x0a, 0x0a, 0x0a];

/// Puts the mark on the taskbar, or takes it off.
///
/// Called from wherever the feed changes, so it follows the bell rather than keeping a count of its
/// own — there is one answer to "is anything unread" and it lives in `Feed`.
pub fn refresh(app: &AppHandle) {
    let unread: usize = app.state::<Feed>().unread_by_entry().values().sum();

    let Ok(window) = webviews::main_window(app) else {
        return;
    };

    #[cfg(target_os = "windows")]
    {
        // Windows has no badge count on a taskbar button; the overlay icon is the badge.
        let _ = window.set_overlay_icon((unread > 0).then(dot));
    }

    #[cfg(not(target_os = "windows"))]
    {
        // macOS and Linux take a number, which is their own idiom, so they get one.
        let _ = window.set_badge_count((unread > 0).then_some(unread as i64));
    }
}

/// A filled circle with a dark rim, as RGBA rows from the top.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn dot() -> tauri::image::Image<'static> {
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];

    let centre = (SIZE as f32 - 1.0) / 2.0;
    // inset rather than full-bleed: the platform scales this whole image into a small corner slot,
    // so a circle that fills the canvas arrives as a blob with no room to read as a badge
    let outer = centre - 3.0;
    let inner = outer - 2.5;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - centre;
            let dy = y as f32 - centre;
            let distance = (dx * dx + dy * dy).sqrt();

            // one pixel of feather at each edge, so the circle does not look cut from a grid
            let coverage = (outer - distance).clamp(0.0, 1.0);
            let fill = (inner - distance).clamp(0.0, 1.0);

            let colour = if fill > 0.0 { DOT } else { RIM };
            let at = ((y * SIZE + x) * 4) as usize;

            rgba[at] = colour[0];
            rgba[at + 1] = colour[1];
            rgba[at + 2] = colour[2];
            rgba[at + 3] = (coverage * 255.0) as u8;
        }
    }

    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads one pixel's alpha out of the raw buffer, which is what the platform is handed.
    fn alpha_at(rgba: &[u8], x: u32, y: u32) -> u8 {
        rgba[((y * SIZE + x) * 4 + 3) as usize]
    }

    fn colour_at(rgba: &[u8], x: u32, y: u32) -> [u8; 3] {
        let at = ((y * SIZE + x) * 4) as usize;

        [rgba[at], rgba[at + 1], rgba[at + 2]]
    }

    #[test]
    fn the_dot_is_a_circle_that_fills_the_icon() {
        let image = dot();
        let rgba = image.rgba();

        assert_eq!(rgba.len(), (SIZE * SIZE * 4) as usize);

        // opaque in the middle, and nothing at all in the corners, or the taskbar gets a square
        assert_eq!(alpha_at(rgba, SIZE / 2, SIZE / 2), 255);
        assert_eq!(alpha_at(rgba, 0, 0), 0);
        assert_eq!(alpha_at(rgba, SIZE - 1, SIZE - 1), 0);
    }

    /// The rim is the whole reason this is not a plain circle: Shiver's own icon is warm orange, and
    /// a red dot on it with no edge is barely a dot.
    ///
    /// Found by scanning rather than by naming a pixel, so changing how far the dot is inset does
    /// not quietly turn this into a test of empty space.
    #[test]
    fn the_dot_has_a_dark_rim_around_the_colour() {
        let image = dot();
        let rgba = image.rgba();

        let first_solid = (0..SIZE)
            .find(|y| alpha_at(rgba, SIZE / 2, *y) == 255)
            .expect("the dot should have an opaque edge somewhere down its middle");

        assert_eq!(
            colour_at(rgba, SIZE / 2, first_solid),
            RIM,
            "the edge is the rim"
        );
        assert_eq!(
            colour_at(rgba, SIZE / 2, SIZE / 2),
            DOT,
            "the middle is the colour"
        );
        assert!(
            first_solid > 0,
            "the dot is inset rather than filling the canvas"
        );
    }
}
