use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, FontId, Frame, RichText, Stroke, Vec2};

// ── Data types ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DetectedItem {
    pub drop_name: String,
    pub platinum: f32,
    pub ducats_ratio: f32,
    pub is_best: bool,
}

pub type DetectionResult = Vec<Option<DetectedItem>>;

// ── App ───────────────────────────────────────────────────────────────────────

pub struct OverlayApp {
    receiver: mpsc::Receiver<DetectionResult>,
    result: DetectionResult,
    display_until: Option<Instant>,
}

impl OverlayApp {
    pub fn new(receiver: mpsc::Receiver<DetectionResult>) -> Self {
        Self {
            receiver,
            result: Vec::new(),
            display_until: None,
        }
    }
}

impl eframe::App for OverlayApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Fully transparent background
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Pass all mouse/keyboard input through to the game
        ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));

        // Pull in the latest detection result (keep only the newest)
        if let Ok(result) = self.receiver.try_recv() {
            self.result = result;
            self.display_until = Some(Instant::now() + Duration::from_secs(30));
            // Actively try to push the window to the very top again when new items arrive
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            
            // Send focus to forcibly bring the X11 window to the front
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);

            // Trigger a native Linux desktop notification as a foolproof fallback
            // since native notifications ALWAYS draw over Exclusive Fullscreen games.
            let mut notify_body = String::new();
            for item_opt in &self.result {
                if let Some(item) = item_opt {
                    notify_body.push_str(&format!("{} ({}p, {}d)\n", item.drop_name, item.platinum, item.ducats_ratio));
                }
            }
            if !notify_body.is_empty() {
                let _ = std::process::Command::new("notify-send")
                    .arg("-a")
                    .arg("WFInfo")
                    .arg("-t")
                    .arg("30000")
                    .arg("-u")
                    .arg("critical")
                    .arg("WFInfo Detected Items")
                    .arg(&notify_body)
                    .spawn();
            }
        }

        // ALWAYS regularly poll the channel so we don't sleep forever while Warframe is running
        ctx.request_repaint_after(Duration::from_millis(250));

        // Check auto-dismiss
        if let Some(until) = self.display_until {
            if Instant::now() >= until {
                self.display_until = None;
                self.result.clear();
            }
        }

        if !self.result.is_empty() && self.display_until.is_some() {
            render_overlay(ctx, &self.result);
        }

        // Keep repainting so we notice new results and the dismiss timer fires
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

const CARD_WIDTH: f32 = 200.0;
const CARD_GAP: f32 = 12.0;

fn render_overlay(ctx: &egui::Context, items: &[Option<DetectedItem>]) {
    let panel_frame = Frame {
        fill: Color32::from_rgba_unmultiplied(10, 10, 15, 210),
        corner_radius: CornerRadius::same(10),
        stroke: Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 30)),
        inner_margin: egui::Margin::same(16),
        outer_margin: egui::Margin { left: 0, right: 0, top: 0, bottom: 8 },
        ..Default::default()
    };

    egui::TopBottomPanel::bottom("overlay_panel")
        .frame(Frame::NONE) // outer panel is transparent
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                panel_frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // A 400px spacer on the left inside a centered container pushes the visible content right by 200px
                        ui.add_space(400.0);
                        ui.spacing_mut().item_spacing = Vec2::new(CARD_GAP, 0.0);

                        for item_opt in items {
                            render_item_card(ui, item_opt);
                        }
                    });
                });
            });
        });
}

fn render_item_card(ui: &mut egui::Ui, item_opt: &Option<DetectedItem>) {
    let card_bg = Color32::from_rgba_unmultiplied(25, 25, 35, 200);
    let card_best_bg = Color32::from_rgba_unmultiplied(20, 50, 30, 220);

    let is_best = item_opt.as_ref().map_or(false, |i| i.is_best);
    let bg = if is_best { card_best_bg } else { card_bg };

    let card_frame = Frame {
        fill: bg,
        corner_radius: CornerRadius::same(8),
        stroke: if is_best {
            Stroke::new(1.5, Color32::from_rgb(80, 200, 100))
        } else {
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 20))
        },
        inner_margin: egui::Margin::same(14),
        outer_margin: egui::Margin::same(0),
        ..Default::default()
    };

    card_frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(CARD_WIDTH);
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 6.0);

            match item_opt {
            Some(item) => {
                // Item name (possibly wraps)
                ui.label(
                    RichText::new(&item.drop_name)
                        .color(Color32::WHITE)
                        .font(FontId::proportional(14.0))
                        .strong(),
                );

                ui.separator();

                // Platinum row
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("⬡")
                            .color(Color32::from_rgb(255, 210, 50))
                            .font(FontId::proportional(13.0)),
                    );
                    ui.label(
                        RichText::new(format!(" {:.0} platinum", item.platinum))
                            .color(Color32::from_rgb(255, 210, 50))
                            .font(FontId::proportional(13.0)),
                    );
                });

                // Ducats row
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("◈")
                            .color(Color32::from_rgb(220, 160, 60))
                            .font(FontId::proportional(13.0)),
                    );
                    ui.label(
                        RichText::new(format!(" {:.0} ducats", item.ducats_ratio))
                            .color(Color32::from_rgb(220, 160, 60))
                            .font(FontId::proportional(13.0)),
                    );
                });

                // Best badge
                if item.is_best {
                    ui.add_space(4.0);
                    let badge_frame = Frame {
                        fill: Color32::from_rgba_unmultiplied(60, 180, 80, 200),
                        corner_radius: CornerRadius::same(4),
                        inner_margin: egui::Margin::symmetric(8, 3),
                        outer_margin: egui::Margin::same(0),
                        ..Default::default()
                    };
                    badge_frame.show(ui, |ui| {
                        ui.label(
                            RichText::new("★  BEST PICK")
                                .color(Color32::WHITE)
                                .font(FontId::proportional(12.0))
                                .strong(),
                        );
                    });
                }
            }
            None => {
                ui.label(
                    RichText::new("Unknown Item")
                        .color(Color32::from_rgb(180, 70, 70))
                        .font(FontId::proportional(14.0))
                        .strong(),
                );
                ui.label(
                    RichText::new("Not found in database")
                        .color(Color32::from_rgb(140, 100, 100))
                        .font(FontId::proportional(12.0)),
                );
            }
        }
        });
    });
}
