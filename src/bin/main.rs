// Overlay moved to library

use std::thread::sleep;
use std::time::Duration;
use std::{error::Error, str::FromStr};
use std::{fs::File, thread};
use std::{
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    sync::mpsc::channel,
};
use std::{path::PathBuf, sync::mpsc};

use clap::Parser;
use eframe::egui;
use env_logger::{Builder, Env};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use image::DynamicImage;
use log::{debug, error, info, warn};
use notify::{watcher, RecursiveMode, Watcher};
use xcap::Window;

use wfinfo::{
    database::Database,
    ocr::{normalize_string, reward_image_to_reward_names, OCR},
    utils::fetch_prices_and_items,
};

use wfinfo::overlay::{DetectedItem, DetectionResult, OverlayApp};

// ── Detection ─────────────────────────────────────────────────────────────────

fn run_detection(capturer: &Window, db: &Database) -> DetectionResult {
    let frame = capturer.capture_image().unwrap();
    info!("Captured");
    let image = DynamicImage::ImageRgba8(frame);
    info!("Converted");
    let text = reward_image_to_reward_names(image, None);
    let text_normalized: Vec<String> = text.iter().map(|s| normalize_string(s)).collect();
    debug!("{:#?}", text_normalized);

    let items: Vec<Option<_>> = text_normalized
        .iter()
        .map(|s| db.find_item(s, None))
        .collect();

    let best = items
        .iter()
        .map(|item| {
            item.map(|item| {
                item.platinum
                    .max(item.ducats as f32 / 10.0 + item.platinum / 100.0)
            })
            .unwrap_or(0.0)
        })
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|best| best.0);

    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            if let Some(item) = item {
                let is_best = Some(index) == best;
                info!(
                    "{}\n\t{}p\t{}d\t{}",
                    item.drop_name,
                    item.platinum,
                    item.ducats as f32 / 10.0,
                    if is_best { "<----" } else { "" }
                );
                Some(DetectedItem {
                    drop_name: item.drop_name.clone(),
                    platinum: item.platinum,
                    ducats_ratio: item.ducats as f32 / 10.0,
                    is_best,
                })
            } else {
                warn!("Unknown item\n\tUnknown");
                None
            }
        })
        .collect()
}

// ── Watchers ──────────────────────────────────────────────────────────────────

fn log_watcher(path: PathBuf, event_sender: mpsc::Sender<()>) {
    debug!("Path: {}", path.display());
    let mut position = File::open(&path)
        .unwrap_or_else(|_| panic!("Couldn't open file {}", path.display()))
        .seek(SeekFrom::End(0))
        .unwrap();

    thread::spawn(move || {
        debug!("Position: {}", position);

        let (tx, rx) = mpsc::channel();
        let mut watcher = watcher(tx, Duration::from_millis(100)).unwrap();
        watcher
            .watch(&path, RecursiveMode::NonRecursive)
            .unwrap_or_else(|_| panic!("Failed to open EE.log file: {}", path.display()));

        loop {
            match rx.recv() {
                Ok(notify::DebouncedEvent::Write(_)) => {
                    let mut f = File::open(&path).unwrap();
                    f.seek(SeekFrom::Start(position)).unwrap();

                    let mut reward_screen_detected = false;

                    let reader = BufReader::new(f.by_ref());
                    for line in reader.lines() {
                        let line = match line {
                            Ok(line) => line,
                            Err(err) => {
                                error!("Error reading line: {}", err);
                                continue;
                            }
                        };
                        if line.contains("Pause countdown done")
                            || line.contains("Got rewards")
                            || line.contains(
                                "Created /Lotus/Interface/ProjectionRewardChoice.swf",
                            )
                        {
                            reward_screen_detected = true;
                        }
                    }

                    if reward_screen_detected {
                        info!("Detected, waiting...");
                        // 750ms wait so the Warframe reward screen finishes its slide-in animation
                        // before we take the picture. (Lowered from 1500ms to increase response time)
                        sleep(Duration::from_millis(750));
                        event_sender.send(()).unwrap();
                    }

                    position = f.metadata().unwrap().len();
                    debug!("Log position: {}", position);
                }
                Ok(_) => {}
                Err(err) => {
                    error!("Error: {:?}", err);
                }
            }
        }
    });
}

fn hotkey_watcher(hotkey: HotKey, event_sender: mpsc::Sender<()>) {
    debug!("watching hotkey: {hotkey:?}");
    thread::spawn(move || {
        let manager = GlobalHotKeyManager::new().unwrap();
        manager.register(hotkey).unwrap();

        while let Ok(event) = GlobalHotKeyEvent::receiver().recv() {
            debug!("{:?}", event);
            if event.state == HotKeyState::Pressed {
                event_sender.send(()).unwrap();
            }
        }
    });
}

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Arguments {
    /// Path to the `EE.log` file located in the game installation directory
    ///
    /// Most likely located at `~/.local/share/Steam/steamapps/compatdata/230410/pfx/drive_c/users/steamuser/AppData/Local/Warframe/EE.log`
    game_log_file_path: Option<PathBuf>,
    /// Warframe Window Name
    ///
    /// some systems may require the window name to be specified (e.g. when using gamescope)
    #[arg(short, long, default_value = "Warframe")]
    window_name: String,
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse();
    let default_log_path = PathBuf::from_str(&std::env::var("HOME").unwrap())
        .unwrap()
        .join(PathBuf::from_str(".local/share/Steam/steamapps/compatdata/230410/pfx/drive_c/users/steamuser/AppData/Local/Warframe/EE.log")?);
    let log_path = arguments.game_log_file_path.unwrap_or(default_log_path);
    let window_name = arguments.window_name;

    let env = Env::default()
        .filter_or("WFINFO_LOG", "info")
        .write_style_or("WFINFO_STYLE", "always");
    Builder::from_env(env)
        .format_timestamp(None)
        .format_level(false)
        .format_module_path(false)
        .format_target(false)
        .init();

    // Find and take ownership of the Warframe window so it can be moved to
    // the detection thread. We specifically pick the largest one to avoid grabbing
    // the tiny Warframe Launcher window if it's still running in the background.
    let mut windows = Window::all()?;
    let warframe_idx = windows
        .iter()
        .enumerate()
        .filter(|(_, x)| x.title() == window_name)
        .max_by_key(|(_, x)| x.width() * x.height())
        .map(|(idx, _)| idx)
        .ok_or("Warframe window not found")?;
    let warframe_window = windows.swap_remove(warframe_idx);

    let win_x = warframe_window.x() as f32;
    let win_y = warframe_window.y() as f32;
    let win_w = warframe_window.width() as f32;
    let win_h = warframe_window.height() as f32;

    println!(
        "Found Warframe window: {}x{} at {},{}",
        win_w, win_h, win_x, win_y
    );

    let (prices, items) = fetch_prices_and_items()?;
    let db = Database::load_from_file(Some(&prices), Some(&items));

    info!("Loaded database");

    // Channels: game events -> detection thread -> overlay UI
    let (event_sender, event_receiver) = channel::<()>();
    let (result_sender, result_receiver) = channel::<DetectionResult>();

    log_watcher(log_path, event_sender.clone());
    hotkey_watcher("F12".parse()?, event_sender);

    // Background thread: wait for events and run OCR detection
    thread::spawn(move || {
        while let Ok(()) = event_receiver.recv() {
            info!("Capturing");
            let result = run_detection(&warframe_window, &db);
            if result_sender.send(result).is_err() {
                break;
            }
        }
        drop(OCR.lock().unwrap().take());
    });

    // Overlay height (accommodates up to 4 reward cards)
    let overlay_h = 180.0_f32;
    // Position at the bottom of the captured window area
    let overlay_y = win_y + win_h - overlay_h;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WFInfo Overlay")
            .with_always_on_top()
            .with_transparent(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size([win_w, overlay_h])
            .with_position([win_x, overlay_y]),
        ..Default::default()
    };

    // Run the egui event loop on the main thread (required on some platforms)
    eframe::run_native(
        "WFInfo Overlay",
        options,
        Box::new(|_cc| Ok(Box::new(OverlayApp::new(result_receiver)))),
    )?;

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;
    use std::fs::read_to_string;

    use image::io::Reader;
    use indexmap::IndexMap;
    use rayon::prelude::*;
    use tesseract::Tesseract;
    use wfinfo::ocr::detect_theme;
    use wfinfo::ocr::extract_parts;
    use wfinfo::testing::Label;

    use super::*;

    #[test]
    fn single_image() {
        let image = Reader::open(format!("test-images/{}.png", 1))
            .unwrap()
            .decode()
            .unwrap();
        let text = reward_image_to_reward_names(image, None);
        let text = text.iter().map(|s| normalize_string(s));
        println!("{:#?}", text);
        let db = Database::load_from_file(None, None);
        let items: Vec<_> = text.map(|s| db.find_item(&s, None)).collect();
        println!("{:#?}", items);

        assert_eq!(
            items[0].expect("Didn't find an item?").drop_name,
            "Octavia Prime Systems Blueprint"
        );
        assert_eq!(
            items[1].expect("Didn't find an item?").drop_name,
            "Octavia Prime Blueprint"
        );
        assert_eq!(
            items[2].expect("Didn't find an item?").drop_name,
            "Tenora Prime Blueprint"
        );
        assert_eq!(
            items[3].expect("Didn't find an item?").drop_name,
            "Harrow Prime Systems Blueprint"
        );
    }

    // #[test]
    #[allow(dead_code)]
    fn wfi_images_exact() {
        let labels: IndexMap<String, Label> =
            serde_json::from_str(&read_to_string("WFI test images/labels.json").unwrap()).unwrap();
        for (filename, label) in labels {
            let image = Reader::open("WFI test images/".to_string() + &filename)
                .unwrap()
                .decode()
                .unwrap();
            let text = reward_image_to_reward_names(image, None);
            let text: Vec<_> = text.iter().map(|s| normalize_string(s)).collect();
            println!("{:#?}", text);

            let db = Database::load_from_file(None, None);
            let items: Vec<_> = text.iter().map(|s| db.find_item(s, None)).collect();
            println!("{:#?}", items);
            println!("{}", filename);

            let item_names = items
                .iter()
                .map(|item| item.map(|item| item.drop_name.clone()));

            for (result, expectation) in item_names.zip(label.items) {
                if expectation.is_empty() {
                    assert_eq!(result, None)
                } else {
                    assert_eq!(result, Some(expectation))
                }
            }
        }
    }

    #[test]
    fn wfi_images_99_percent() {
        let labels: BTreeMap<String, Label> =
            serde_json::from_str(&read_to_string("WFI test images/labels.json").unwrap()).unwrap();
        let total = labels.len();
        let success_count: usize = labels
            .into_par_iter()
            .map(|(filename, label)| {
                let image = Reader::open("WFI test images/".to_string() + &filename)
                    .unwrap()
                    .decode()
                    .unwrap();
                let text = reward_image_to_reward_names(image, None);
                let text: Vec<_> = text.iter().map(|s| normalize_string(s)).collect();
                println!("{:#?}", text);

                let db = Database::load_from_file(None, None);
                let items: Vec<_> = text.iter().map(|s| db.find_item(s, None)).collect();
                println!("{:#?}", items);
                println!("{}", filename);

                let item_names = items
                    .iter()
                    .map(|item| item.map(|item| item.drop_name.clone()));

                if item_names.zip(label.items).all(|(result, expectation)| {
                    expectation == result.unwrap_or_else(|| "".to_string())
                }) {
                    1
                } else {
                    0
                }
            })
            .sum();

        let success_rate = success_count as f32 / total as f32;
        assert!(success_rate > 0.95, "Success rate: {success_rate}");
    }

    // #[test]
    #[allow(dead_code)]
    fn images() {
        let tests = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];
        for i in tests {
            let image = Reader::open(format!("test-images/{}.png", i))
                .unwrap()
                .decode()
                .unwrap();

            let theme = detect_theme(&image);
            println!("Theme: {:?}", theme);

            let parts = extract_parts(&image, theme);

            let mut ocr =
                Tesseract::new(None, Some("eng")).expect("Could not initialize Tesseract");
            for part in parts {
                let buffer = part.as_flat_samples_u8().unwrap();
                ocr = ocr
                    .set_frame(
                        buffer.samples,
                        part.width() as i32,
                        part.height() as i32,
                        3,
                        3 * part.width() as i32,
                    )
                    .expect("Failed to set image");
                let text = ocr.get_text().expect("Failed to get text");
                println!("{}", text);
            }
            println!("=================");
        }
    }
}
