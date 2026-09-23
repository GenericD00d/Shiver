//! The unread mark on Shiver's taskbar icon: a dot on Windows (overlay icon), a count elsewhere.

use tauri::{AppHandle, Manager};

use crate::webviews;

/// Side of the overlay icon; Windows scales it into the taskbar button's corner.
const SIZE: u32 = 32;

/// `--shiver-danger`, the colour of the rail's unread badge.
const DOT: [u8; 3] = [0xff, 0x64, 0x67];

/// Shows the same total the rail does: feed notifications plus what arrived while Shiver was closed.
pub fn refresh<R: tauri::Runtime>(app: &AppHandle<R>) {
    let unread = app.state::<crate::feed::Feed>().unread_count()
        + app.state::<crate::watch::Missed>().total();

    let Ok(window) = webviews::main_window(app) else {
        return;
    };

    #[cfg(target_os = "windows")]
    let _ = window.set_overlay_icon((unread > 0).then(dot));

    #[cfg(not(target_os = "windows"))]
    let _ = window.set_badge_count((unread > 0).then_some(unread as i64));
}

/// A filled, anti-aliased circle in `DOT`, inset from the canvas edge, as RGBA.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn dot() -> tauri::image::Image<'static> {
    let centre = (SIZE as f32 - 1.0) / 2.0;
    let radius = centre - 3.0;

    let rgba = (0..SIZE * SIZE)
        .flat_map(|index| {
            let (x, y) = (
                (index % SIZE) as f32 - centre,
                (index / SIZE) as f32 - centre,
            );
            let coverage = (radius - (x * x + y * y).sqrt()).clamp(0.0, 1.0);

            [DOT[0], DOT[1], DOT[2], (coverage * 255.0) as u8]
        })
        .collect();

    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_is_one_colour_opaque_in_the_middle_and_clear_in_the_corners() {
        let image = dot();
        let rgba = image.rgba();
        let pixel = |x: u32, y: u32| &rgba[((y * SIZE + x) * 4) as usize..][..4];

        assert_eq!(rgba.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(pixel(SIZE / 2, SIZE / 2), [DOT[0], DOT[1], DOT[2], 255]);
        assert_eq!(pixel(0, 0)[3], 0);
        assert_eq!(pixel(SIZE - 1, SIZE - 1)[3], 0);
    }
}
