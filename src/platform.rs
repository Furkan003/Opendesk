use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use opendesk::network::{InputEvent, MonitorInfo};
#[cfg(target_os = "linux")]
#[path = "wayland.rs"]
mod wayland;

pub fn start_capture() {
    #[cfg(target_os = "linux")]
    wayland::start();
}
pub fn stop_capture() {
    #[cfg(target_os = "linux")]
    wayland::stop();
}
pub fn display_list() -> Result<Vec<MonitorInfo>, String> {
    #[cfg(target_os = "linux")]
    if wayland::active() {
        let (width, height) = wayland::current_dimensions().unwrap_or((0, 0));
        return Ok(vec![MonitorInfo {
            id: 0,
            name: "Monitor selected in the Wayland sharing dialog".into(),
            width,
            height,
        }]);
    }
    screenshots::Screen::all()
        .map_err(|e| e.to_string())
        .map(|screens| {
            screens
                .into_iter()
                .enumerate()
                .map(|(id, s)| MonitorInfo {
                    id,
                    name: format!(
                        "Display {}{}",
                        id + 1,
                        if s.display_info.is_primary {
                            " (primary)"
                        } else {
                            ""
                        }
                    ),
                    width: s.display_info.width,
                    height: s.display_info.height,
                })
                .collect()
        })
}
pub fn capture_jpeg() -> Result<Vec<u8>, String> {
    capture_display(0, 65)
}
pub fn capture_display(index: usize, quality: u8) -> Result<Vec<u8>, String> {
    capture_display_scaled(index, quality, 0)
}
pub fn capture_display_scaled(
    index: usize,
    quality: u8,
    max_width: u32,
) -> Result<Vec<u8>, String> {
    std::panic::catch_unwind(|| capture_inner(index, quality.clamp(10, 95), max_width))
        .map_err(|_| "Screen capture provider failed; sharing stopped".to_owned())?
}
fn capture_inner(index: usize, quality: u8, max_width: u32) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "linux")]
    if wayland::active() {
        if index != 0 {
            return Err("Select the monitor in the compositor sharing dialog".into());
        }
        let jpeg = wayland::capture()?;
        let image = image::load_from_memory(&jpeg)
            .map_err(|e| e.to_string())?
            .to_rgb8();
        return encode(
            image.as_raw(),
            image.width(),
            image.height(),
            quality,
            max_width,
        );
    }
    let screen = screenshots::Screen::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .nth(index)
        .ok_or("Selected display is no longer available")?;
    let shot = screen.capture().map_err(|e| e.to_string())?;
    let rgb: Vec<u8> = shot
        .as_raw()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    encode(&rgb, shot.width(), shot.height(), quality, max_width)
}
fn encode(
    rgb: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    max_width: u32,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality);
    if max_width > 0 && width > max_width {
        let raw = image::RgbImage::from_raw(width, height, rgb.to_vec())
            .ok_or("Invalid capture dimensions")?;
        let target_height = ((height as u64 * max_width as u64 / width as u64) as u32).max(1);
        let scaled = image::imageops::resize(
            &raw,
            max_width,
            target_height,
            image::imageops::FilterType::Triangle,
        );
        encoder
            .encode(
                scaled.as_raw(),
                max_width,
                target_height,
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| e.to_string())?;
    } else {
        encoder
            .encode(rgb, width, height, image::ExtendedColorType::Rgb8)
            .map_err(|e| e.to_string())?;
    }
    Ok(output)
}

pub struct InputController {
    enigo: Option<Enigo>,
    held: Vec<InputEvent>,
    monitor: usize,
}
impl InputController {
    pub fn new(monitor: usize) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        if wayland::active() {
            if monitor != 0 {
                return Err("Wayland monitor is selected by the compositor dialog".into());
            }
            wayland::prepare_async();
            return Ok(Self {
                enigo: None,
                held: Vec::new(),
                monitor,
            });
        }
        Ok(Self {
            enigo: Some(Enigo::new(&Settings::default()).map_err(|e| e.to_string())?),
            held: Vec::new(),
            monitor,
        })
    }
    pub fn apply(&mut self, event: InputEvent) -> Result<(), String> {
        if matches!(event, InputEvent::ReleaseAll) {
            self.release_all();
            return Ok(());
        }
        self.inject(event.clone())?;
        match &event {
            InputEvent::Key { key, pressed } => {
                self.held
                    .retain(|e| !matches!(e,InputEvent::Key{key:k,..}if k==key));
                if *pressed {
                    self.held.push(event);
                }
            }
            InputEvent::Button { button, pressed } => {
                self.held
                    .retain(|e| !matches!(e,InputEvent::Button{button:b,..}if b==button));
                if *pressed {
                    self.held.push(event);
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn inject(&mut self, event: InputEvent) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        if self.enigo.is_none() {
            return wayland::input(event);
        }
        let enigo = self.enigo.as_mut().ok_or("Input controller unavailable")?;
        let direction = |pressed| {
            if pressed {
                Direction::Press
            } else {
                Direction::Release
            }
        };
        match event {
            InputEvent::ReleaseAll => return Ok(()),
            InputEvent::Move { x, y } => {
                if !x.is_finite() || !y.is_finite() {
                    return Err("Invalid pointer coordinate".into());
                }
                let screen = screenshots::Screen::all()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .nth(self.monitor)
                    .ok_or("No selected display")?;
                enigo.move_mouse(
                    screen.display_info.x
                        + (x.clamp(0.0, 1.0) * screen.display_info.width.saturating_sub(1) as f32)
                            as i32,
                    screen.display_info.y
                        + (y.clamp(0.0, 1.0) * screen.display_info.height.saturating_sub(1) as f32)
                            as i32,
                    Coordinate::Abs,
                )
            }
            InputEvent::Button { button, pressed } => enigo.button(
                match button {
                    0 => Button::Left,
                    1 => Button::Right,
                    2 => Button::Middle,
                    _ => return Err("Unsupported pointer button".into()),
                },
                direction(pressed),
            ),
            InputEvent::Scroll { x, y } => enigo
                .scroll(x, Axis::Horizontal)
                .and_then(|_| enigo.scroll(y, Axis::Vertical)),
            InputEvent::Text(text) => enigo.text(&text),
            InputEvent::Key { key, pressed } => enigo.key(
                map_key(&key).ok_or_else(|| format!("Unsupported key: {key}"))?,
                direction(pressed),
            ),
        }
        .map_err(|e| e.to_string())
    }
    pub fn release_all(&mut self) {
        for event in std::mem::take(&mut self.held) {
            let release = match event {
                InputEvent::Key { key, .. } => InputEvent::Key {
                    key,
                    pressed: false,
                },
                InputEvent::Button { button, .. } => InputEvent::Button {
                    button,
                    pressed: false,
                },
                _ => continue,
            };
            let _ = self.inject(release);
        }
    }
}
impl Drop for InputController {
    fn drop(&mut self) {
        self.release_all();
    }
}
fn map_key(key: &str) -> Option<Key> {
    Some(match key {
        "Enter" => Key::Return,
        "Escape" => Key::Escape,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Insert" => Key::Insert,
        "ArrowUp" => Key::UpArrow,
        "ArrowDown" => Key::DownArrow,
        "ArrowLeft" => Key::LeftArrow,
        "ArrowRight" => Key::RightArrow,
        "Space" => Key::Space,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Control" | "Ctrl" => Key::Control,
        "Shift" => Key::Shift,
        "Alt" => Key::Alt,
        "Meta" | "Super" | "Command" => Key::Meta,
        "CapsLock" => Key::CapsLock,
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        "F13" => Key::F13,
        "F14" => Key::F14,
        "F15" => Key::F15,
        "F16" => Key::F16,
        "F17" => Key::F17,
        "F18" => Key::F18,
        "F19" => Key::F19,
        "F20" => Key::F20,
        "Num0" => Key::Unicode('0'),
        "Num1" => Key::Unicode('1'),
        "Num2" => Key::Unicode('2'),
        "Num3" => Key::Unicode('3'),
        "Num4" => Key::Unicode('4'),
        "Num5" => Key::Unicode('5'),
        "Num6" => Key::Unicode('6'),
        "Num7" => Key::Unicode('7'),
        "Num8" => Key::Unicode('8'),
        "Num9" => Key::Unicode('9'),
        "Minus" => Key::Unicode('-'),
        "Plus" | "Equals" => Key::Unicode('='),
        "Comma" => Key::Unicode(','),
        "Period" => Key::Unicode('.'),
        "Slash" => Key::Unicode('/'),
        "Backslash" => Key::Unicode('\\'),
        "Semicolon" | "Colon" => Key::Unicode(';'),
        "Quote" => Key::Unicode('\''),
        "Backtick" => Key::Unicode('`'),
        "OpenBracket" => Key::Unicode('['),
        "CloseBracket" => Key::Unicode(']'),
        _ => {
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() || c.is_control() {
                return None;
            }
            Key::Unicode(c.to_ascii_lowercase())
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_shortcuts_and_function_keys_are_mapped() {
        for key in [
            "F1",
            "F12",
            "F20",
            "Num0",
            "9",
            "Meta",
            "Insert",
            "Backslash",
            "ş",
        ] {
            assert!(map_key(key).is_some(), "{key}");
        }
        assert!(map_key("UnsupportedKey").is_none());
    }
    #[test]
    fn scaled_capture_preserves_aspect_ratio() {
        let rgb = vec![128; 320 * 200 * 3];
        let encoded = encode(&rgb, 320, 200, 75, 160).unwrap();
        let decoded = image::load_from_memory(&encoded).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (160, 100));
    }
}
