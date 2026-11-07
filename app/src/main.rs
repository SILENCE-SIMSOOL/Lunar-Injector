#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(not(windows))]
fn main() {
	eprintln!("Lunar Injector requires Windows 10/11 x64.");
}

#[cfg(windows)]
fn main() {
	if let Err(error) = launch() {
		let message: Vec<u16> = format!("Cannot start Lunar Injector:\n{error}\0").encode_utf16().collect();
		let title: Vec<u16> = "Lunar Injector\0".encode_utf16().collect();
		unsafe {
			windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
				0, message.as_ptr(), title.as_ptr(),
				windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
					| windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
			);
		}
	}
}

#[cfg(windows)]
fn launch() -> eframe::Result {
	let icon_bytes = include_bytes!("../assets/icon_64.rgba");
	let icon = egui::IconData {
		rgba: icon_bytes.to_vec(),
		width: 64,
		height: 64,
	};
	let options = eframe::NativeOptions {
		viewport: eframe::egui::ViewportBuilder::default()
			.with_inner_size([1024.0, 576.0])
			.with_min_inner_size([1024.0, 576.0])
			.with_max_inner_size([1024.0, 576.0])
			.with_resizable(false)
			.with_decorations(false)
			.with_icon(icon)
			.with_title("Lunar Injector"),
		..Default::default()
	};
	eframe::run_native("Lunar Injector", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[cfg(windows)]
use eframe::egui::{self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
#[cfg(windows)]
use lunar_shared::{Config, Mode, Status, ITEMS};
#[cfg(windows)]
use std::collections::HashSet;
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
const BG: Color32 = Color32::from_rgb(14, 15, 18);
#[cfg(windows)]
const TEXT: Color32 = Color32::from_rgb(245, 245, 247);
#[cfg(windows)]
const MUTED: Color32 = Color32::from_rgb(145, 148, 156);
#[cfg(windows)]
const LINE: Color32 = Color32::from_rgb(42, 44, 50);
#[cfg(windows)]
const CONTROL_BG: Color32 = Color32::from_rgb(20, 21, 25);
#[cfg(windows)]
const CONTROL_BORDER: Color32 = Color32::from_rgb(45, 47, 54);
#[cfg(windows)]
const GREEN_DOT: Color32 = Color32::from_rgb(52, 216, 118);

#[cfg(windows)]
struct App {
	config: Config,
	clients: Vec<win::ClientWindow>,
	selected: Option<u32>,
	status: Option<Status>,
	message: String,
	waiting_key: bool,
	last_poll: Instant,
	last_client_scan: Instant,
	maximized: bool,
	pending_injection: Option<(u32, Receiver<Result<(), String>>)>,
	loaded_pids: HashSet<u32>,
	connection_attempt: Option<(u32, Instant)>,
	icon_texture: egui::TextureHandle,
}

#[cfg(windows)]
impl App {
	fn new(cc: &eframe::CreationContext<'_>) -> Self {
		setup_fonts(&cc.egui_ctx);
		let icon_bytes = include_bytes!("../assets/icon_64.rgba");
		let color_image = egui::ColorImage::from_rgba_unmultiplied([64, 64], icon_bytes);
		let icon_texture = cc.egui_ctx.load_texture("app_icon", color_image, egui::TextureOptions::LINEAR);
		let mut style = (*cc.egui_ctx.style()).clone();
		style.visuals = egui::Visuals::dark();
		style.visuals.panel_fill = BG;
		style.visuals.window_fill = BG;
		style.visuals.override_text_color = Some(TEXT);
		style.visuals.widgets.noninteractive.bg_fill = CONTROL_BG;
		style.visuals.widgets.inactive.bg_fill = CONTROL_BG;
		style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(32, 34, 40);
		style.visuals.widgets.active.bg_fill = Color32::from_rgb(48, 50, 58);
		style.visuals.widgets.inactive.fg_stroke.color = TEXT;
		style.visuals.selection.bg_fill = Color32::WHITE;
		style.visuals.selection.stroke = Stroke::new(1.0_f32, BG);
		style.visuals.hyperlink_color = TEXT;
		cc.egui_ctx.set_style(style);

		let clients = win::detect();
		let selected = clients.first().map(|client| client.pid);
		Self {
			config: lunar_shared::load_config(),
			clients,
			selected,
			status: None,
			message: String::new(),
			waiting_key: false,
			last_poll: Instant::now() - Duration::from_secs(5),
			last_client_scan: Instant::now(),
			maximized: false,
			pending_injection: None,
			loaded_pids: HashSet::new(),
			connection_attempt: None,
			icon_texture,
		}
	}

	fn scan_clients(&mut self) {
		if self.last_client_scan.elapsed() < Duration::from_secs(2) {
			return;
		}
		self.last_client_scan = Instant::now();
		self.clients = win::detect();
		self.loaded_pids.retain(|pid| self.clients.iter().any(|window| window.pid == *pid));
		if !self.clients.iter().any(|window| Some(window.pid) == self.selected) {
			self.selected = self.clients.first().map(|window| window.pid);
		}
	}

	fn poll(&mut self) {
		if self.last_poll.elapsed() < Duration::from_millis(400) {
			return;
		}
		self.last_poll = Instant::now();
		self.status = self.selected.and_then(|pid| {
			let path = lunar_shared::status_path(pid);
			let recent = path.metadata().ok()?.modified().ok()?.elapsed().ok()? < Duration::from_secs(2);
			if !recent {
				return None;
			}
			serde_json::from_slice::<Status>(&std::fs::read(path).ok()?).ok()
		});
	}

	fn check_injection(&mut self) {
		let completed = self.pending_injection.as_ref().and_then(|(pid, receiver)| {
			match receiver.try_recv() {
				Ok(result) => Some((*pid, result)),
				Err(TryRecvError::Disconnected) => Some((*pid, Err("Injection worker stopped unexpectedly".into()))),
				Err(TryRecvError::Empty) => None,
			}
		});
		if let Some((pid, result)) = completed {
			self.pending_injection = None;
			self.message = match result {
				Ok(()) => {
					self.loaded_pids.insert(pid);
					self.connection_attempt = Some((pid, Instant::now()));
					"DLL loaded; checking the bridge connection...".into()
				}
				Err(error) => error,
			};
		}
		if let Some((pid, since)) = self.connection_attempt {
			if self.selected == Some(pid) && self.status.as_ref().is_some_and(|s| s.ready && s.pid == pid) {
				self.connection_attempt = None;
			} else if since.elapsed() > Duration::from_secs(6) {
				if self.selected == Some(pid) {
					self.message = "Bridge did not connect. Restart Lunar Client to retry.".into();
				}
				self.connection_attempt = None;
			}
		}
	}

	fn inject(&mut self, pid: u32) {
		let dll = std::env::current_exe().ok().and_then(|exe| {
			exe.parent().map(|directory| directory.join("lunar_bridge.dll"))
		});
		let Some(dll) = dll else {
			self.message = "Cannot locate the injector directory".into();
			return;
		};
		let stale_status = lunar_shared::status_path(pid);
		if let Err(error) = std::fs::remove_file(stale_status) {
			if error.kind() != std::io::ErrorKind::NotFound {
				self.message = format!("Cannot clear old bridge status: {error}");
				return;
			}
		}
		self.status = None;
		self.message = "Loading DLL...".into();
		let (sender, receiver) = mpsc::channel();
		self.pending_injection = Some((pid, receiver));
		std::thread::spawn(move || {
			let _ = sender.send(win::inject(pid, &dll));
		});
	}

	fn select_key(&mut self, ctx: &egui::Context) {
		if !self.waiting_key {
			return;
		}
		let key = ctx.input(|input| input.events.iter().find_map(|event| match event {
			egui::Event::Key { key, pressed: true, .. } => Some(*key),
			_ => None,
		}));
		if let Some(key) = key {
			if key == egui::Key::Escape {
				self.waiting_key = false;
			} else if let Some(vk) = vk_for_key(key) {
				self.config.right_key = vk;
				self.waiting_key = false;
			}
		}
	}

	fn render_title_bar(&mut self, ctx: &egui::Context) {
		egui::TopBottomPanel::top("title_bar")
			.exact_height(54.0)
			.frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(38, 0)))
			.show(ctx, |ui| {
				let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 54.0), Sense::hover());
				let drag_rect = Rect::from_min_max(rect.min, Pos2::new(rect.right() - 120.0, rect.bottom()));
				let drag = ui.interact(drag_rect, ui.id().with("window_drag"), Sense::click_and_drag());
				if drag.drag_started() {
					ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
				}
				if drag.double_clicked() {
					self.maximized = !self.maximized;
					ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(self.maximized));
				}

				let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + 16.0, rect.center().y), egui::vec2(24.0, 24.0));
				ui.painter().image(
					self.icon_texture.id(),
					icon_rect,
					Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
					Color32::WHITE,
				);
				label_semibold(ui, rect.left() + 38.0, rect.center().y, "Lunar Injector", 19.5, TEXT);

				let button_width = 38.0;
				let button_height = 32.0;
				let right_start = rect.right() - button_width * 3.0;

				for index in 0..3 {
					let button = Rect::from_min_size(
						Pos2::new(right_start + index as f32 * button_width, rect.center().y - button_height * 0.5),
						egui::vec2(button_width, button_height),
					);
					let response = ui.interact(button, ui.id().with(("window_button", index)), Sense::click());
					if response.hovered() {
						ui.painter().rect_filled(button, 4.0, Color32::from_rgba_unmultiplied(255, 255, 255, 18));
					}
					let center = button.center();
					match index {
						0 => {
							ui.painter().line_segment(
								[Pos2::new(center.x - 5.5, center.y), Pos2::new(center.x + 5.5, center.y)],
								Stroke::new(1.2_f32, TEXT),
							);
						}
						1 => {
							let box_rect = Rect::from_center_size(center, egui::vec2(10.0, 10.0));
							ui.painter().rect_stroke(box_rect, 1.0, Stroke::new(1.2_f32, TEXT), StrokeKind::Inside);
						}
						_ => {
							ui.painter().line_segment(
								[Pos2::new(center.x - 4.5, center.y - 4.5), Pos2::new(center.x + 4.5, center.y + 4.5)],
								Stroke::new(1.2_f32, TEXT),
							);
							ui.painter().line_segment(
								[Pos2::new(center.x + 4.5, center.y - 4.5), Pos2::new(center.x - 4.5, center.y + 4.5)],
								Stroke::new(1.2_f32, TEXT),
							);
						}
					}
					if response.clicked() {
						match index {
							0 => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
							1 => {
								self.maximized = !self.maximized;
								ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(self.maximized));
							}
							_ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
						}
					}
				}
				ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0_f32, LINE));
			});
	}

	fn render_process(&mut self, ui: &mut egui::Ui) {
		let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), Sense::hover());
		let detected = self.selected.is_some();
		let dot = if detected { GREEN_DOT } else { MUTED };
		ui.painter().circle_filled(Pos2::new(rect.left() + 16.0, rect.center().y), 6.0, dot);

		let client_name = self.clients.iter().find(|w| Some(w.pid) == self.selected)
			.map(|_| "Lunar Client 1.8.9").unwrap_or("Lunar Client not detected");
		label_semibold(ui, rect.left() + 38.0, rect.center().y, client_name, 17.5, TEXT);

		let connected = self.status.as_ref().is_some_and(|status| status.ready && Some(status.pid) == self.selected);
		let pending = self.pending_injection.is_some();
		let loaded = self.selected.is_some_and(|pid| self.loaded_pids.contains(&pid));
		let caption = if connected { "Injected" } else if pending { "Injecting..." }
			else if loaded { "Restart Client" } else { "Inject" };

		let button_rect = Rect::from_min_size(
			Pos2::new(rect.right() - 176.0, rect.center().y - 19.0),
			egui::vec2(176.0, 38.0),
		);
		let enabled = detected && !loaded && !connected && !pending;
		let response = ui.interact(button_rect, ui.id().with("inject"), if enabled { Sense::click() } else { Sense::hover() });

		let button_fill = if !enabled {
			Color32::from_rgb(52, 54, 60)
		} else if response.hovered() {
			Color32::from_rgb(238, 240, 243)
		} else {
			Color32::WHITE
		};
		let button_text_color = if enabled { BG } else { MUTED };

		ui.painter().rect_filled(button_rect, 9.0, button_fill);
		let syringe_center = Pos2::new(button_rect.center().x - 34.0, button_rect.center().y);
		draw_syringe(ui.painter(), syringe_center, button_text_color);
		let font = FontId::new(17.0, FontFamily::Name("Inter-SemiBold".into()));
		ui.painter().text(
			Pos2::new(button_rect.center().x + 8.0, button_rect.center().y),
			Align2::CENTER_CENTER,
			caption,
			font,
			button_text_color,
		);

		if response.clicked() && enabled {
			if let Some(pid) = self.selected {
				self.inject(pid);
			}
		}
	}

	fn render_modules(&mut self, ui: &mut egui::Ui) {
		let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 55.0), Sense::hover());
		let middle = rect.left() + 284.0;

		label_semibold(ui, rect.left() + 14.0, rect.center().y, "HitDelayFix", 18.0, TEXT);
		toggle(
			ui,
			Rect::from_min_size(Pos2::new(rect.left() + 138.0, rect.center().y - 12.0), egui::vec2(49.0, 24.0)),
			&mut self.config.hit_delay_fix,
			"hit_delay_fix",
		);

		ui.painter().line_segment(
			[Pos2::new(middle, rect.center().y - 13.0), Pos2::new(middle, rect.center().y + 13.0)],
			Stroke::new(1.0_f32, LINE),
		);

		label_semibold(ui, middle + 85.0, rect.center().y, "AutoClicker", 18.0, TEXT);
		toggle(
			ui,
			Rect::from_min_size(Pos2::new(middle + 209.0, rect.center().y - 12.0), egui::vec2(49.0, 24.0)),
			&mut self.config.auto_clicker,
			"auto_clicker",
		);
	}

	fn render_click_rate(&mut self, ui: &mut egui::Ui) {
		let (header, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 54.0), Sense::hover());
		label_semibold(ui, header.left() + 14.0, header.center().y, "Click Rate", 18.0, TEXT);

		let segment = Rect::from_min_size(Pos2::new(header.left() + 148.0, header.center().y - 15.0), egui::vec2(202.0, 30.0));
		ui.painter().rect_filled(segment, 15.0, CONTROL_BG);
		ui.painter().rect_stroke(segment, 15.0, Stroke::new(1.0_f32, CONTROL_BORDER), StrokeKind::Inside);

		let choices = [(Mode::Range, "Range"), (Mode::Graph, "Graph")];
		let item_w = segment.width() * 0.5;
		for (index, (mode, text)) in choices.into_iter().enumerate() {
			let segment_rect = Rect::from_min_size(Pos2::new(segment.left() + index as f32 * item_w, segment.top()), egui::vec2(item_w, 30.0));
			let response = ui.interact(segment_rect, ui.id().with(("mode", index)), Sense::click());
			if response.clicked() {
				self.config.mode = mode;
			}
			let is_selected = self.config.mode == mode;
			if is_selected {
				ui.painter().rect_filled(segment_rect, 15.0, Color32::WHITE);
			}
			let font = if is_selected {
				FontId::new(13.5, FontFamily::Name("Inter-SemiBold".into()))
			} else {
				FontId::new(13.5, FontFamily::Proportional)
			};
			ui.painter().text(
				segment_rect.center(),
				Align2::CENTER_CENTER,
				text,
				font,
				if is_selected { BG } else { MUTED },
			);
		}

		let summary = match self.config.mode {
			Mode::Range => "10 – 20 CPS",
			Mode::Graph => "10 – 20 CPS   |   Total 100%",
		};
		let font = FontId::new(14.0, FontFamily::Proportional);
		ui.painter().text(Pos2::new(header.right() - 8.0, header.center().y), Align2::RIGHT_CENTER, summary, font, MUTED);

		match self.config.mode {
			Mode::Range => self.render_range(ui),
			Mode::Graph => self.render_graph(ui),
		}
	}

	fn render_range(&mut self, ui: &mut egui::Ui) {
		let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 190.0), Sense::hover());
		label_regular(ui, rect.left() + 18.0, rect.top() + 45.0, "Minimum CPS", 15.0, TEXT);
		label_regular(ui, rect.left() + 290.0, rect.top() + 45.0, "Maximum CPS", 15.0, TEXT);
		let min = Rect::from_min_size(Pos2::new(rect.left() + 140.0, rect.top() + 30.0), egui::vec2(90.0, 32.0));
		let max = Rect::from_min_size(Pos2::new(rect.left() + 415.0, rect.top() + 30.0), egui::vec2(90.0, 32.0));
		for input in [min, max] {
			ui.painter().rect_filled(input, 6.0, CONTROL_BG);
			ui.painter().rect_stroke(input, 6.0, Stroke::new(1.0_f32, CONTROL_BORDER), StrokeKind::Inside);
		}
		ui.place(min.shrink(3.0), egui::DragValue::new(&mut self.config.min_cps).range(10..=20).speed(0.2));
		ui.place(max.shrink(3.0), egui::DragValue::new(&mut self.config.max_cps).range(10..=20).speed(0.2));
		self.config.sanitize();
	}

	fn render_graph(&mut self, ui: &mut egui::Ui) {
		let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 190.0), Sense::hover());
		let baseline = rect.bottom() - 45.0;
		let line_top = rect.top() + 48.0;
		let track_height = baseline - line_top;
		let scale = 100.0_f32;

		let start_x = rect.left() + 54.0;
		let end_x = rect.left() + 894.0;
		let column_spacing = (end_x - start_x) / 10.0;

		ui.painter().line_segment(
			[Pos2::new(rect.left() + 24.0, baseline), Pos2::new(rect.right() - 20.0, baseline)],
			Stroke::new(1.0_f32, LINE),
		);

		for index in 0..11 {
			let x = start_x + index as f32 * column_spacing;
			let box_rect = Rect::from_center_size(Pos2::new(x, rect.top() + 18.0), egui::vec2(44.0, 24.0));
			let response = ui.interact(box_rect, ui.id().with(("graph_box", index)), Sense::click_and_drag());

			if response.dragged() {
				let delta = -response.drag_delta().y;
				if delta.abs() >= 1.0 {
					let cur = self.config.graph[index] as i32 + delta.signum() as i32;
					self.config.set_probability(index, cur.clamp(0, 100) as u8);
				}
			}

			ui.painter().rect_filled(box_rect, 5.0, CONTROL_BG);
			ui.painter().rect_stroke(box_rect, 5.0, Stroke::new(1.0_f32, CONTROL_BORDER), StrokeKind::Inside);

			let val_str = format!("{}", self.config.graph[index]);
			let font_bold = FontId::new(12.5, FontFamily::Name("Inter-SemiBold".into()));
			ui.painter().text(
				Pos2::new(box_rect.left() + 11.0, box_rect.center().y),
				Align2::CENTER_CENTER,
				val_str,
				font_bold,
				TEXT,
			);

			let arrows_center = Pos2::new(box_rect.right() - 11.0, box_rect.center().y);
			let up_tri = [
				Pos2::new(arrows_center.x, arrows_center.y - 5.5),
				Pos2::new(arrows_center.x - 3.2, arrows_center.y - 1.5),
				Pos2::new(arrows_center.x + 3.2, arrows_center.y - 1.5),
			];
			ui.painter().add(egui::Shape::convex_polygon(up_tri.to_vec(), MUTED, Stroke::NONE));
			let down_tri = [
				Pos2::new(arrows_center.x, arrows_center.y + 5.5),
				Pos2::new(arrows_center.x - 3.2, arrows_center.y + 1.5),
				Pos2::new(arrows_center.x + 3.2, arrows_center.y + 1.5),
			];
			ui.painter().add(egui::Shape::convex_polygon(down_tri.to_vec(), MUTED, Stroke::NONE));

			let font_reg = FontId::new(12.5, FontFamily::Proportional);
			ui.painter().text(
				Pos2::new(box_rect.right() + 8.0, box_rect.center().y),
				Align2::LEFT_CENTER,
				"%",
				font_reg,
				MUTED,
			);

			let track = Rect::from_min_max(Pos2::new(x - 14.0, line_top), Pos2::new(x + 14.0, baseline));
			let track_resp = ui.interact(track, ui.id().with(("graph_track", index)), Sense::click_and_drag());
			if (track_resp.dragged() || track_resp.clicked()) && ui.ctx().pointer_interact_pos().is_some() {
				let pointer = ui.ctx().pointer_interact_pos().unwrap();
				let pct = ((baseline - pointer.y) / track_height * scale).round().clamp(0.0, 100.0) as u8;
				self.config.set_probability(index, pct);
			}

			let value = self.config.graph[index] as f32;
			let y = baseline - track_height * (value / scale).clamp(0.0, 1.0);

			ui.painter().line_segment(
				[Pos2::new(x, line_top), Pos2::new(x, baseline)],
				Stroke::new(2.0_f32, Color32::from_rgb(46, 48, 54)),
			);
			ui.painter().circle_filled(Pos2::new(x, y), 6.5, Color32::WHITE);

			let label_font = FontId::new(13.5, FontFamily::Proportional);
			ui.painter().text(
				Pos2::new(x, baseline + 18.0),
				Align2::CENTER_CENTER,
				format!("{}", index + 10),
				label_font,
				Color32::from_rgb(228, 230, 235),
			);
		}

		let cps_font = FontId::new(13.0, FontFamily::Proportional);
		ui.painter().text(
			Pos2::new(end_x + 32.0, baseline + 18.0),
			Align2::LEFT_CENTER,
			"CPS",
			cps_font,
			MUTED,
		);
	}

	fn render_conditions(&mut self, ui: &mut egui::Ui) {
		let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 156.0), Sense::hover());
		let middle = rect.left() + 416.0;

		ui.painter().line_segment(
			[Pos2::new(middle, rect.top() + 18.0), Pos2::new(middle, rect.bottom() - 14.0)],
			Stroke::new(1.0_f32, LINE),
		);

		label_semibold(ui, rect.left() + 14.0, rect.top() + 26.0, "Conditions", 18.0, TEXT);
		checkbox(
			ui,
			Rect::from_min_size(Pos2::new(rect.left() + 16.0, rect.top() + 56.0), egui::vec2(340.0, 24.0)),
			"Disable when targeting a block",
			&mut self.config.block_check,
		);

		checkbox(
			ui,
			Rect::from_min_size(Pos2::new(rect.left() + 16.0, rect.top() + 98.0), egui::vec2(180.0, 24.0)),
			"Activate with a key",
			&mut self.config.activate_with_key,
		);

		let key_rect = Rect::from_min_size(Pos2::new(rect.left() + 214.0, rect.top() + 94.0), egui::vec2(80.0, 28.0));
		let key_resp = ui.interact(key_rect, ui.id().with("key_button"), Sense::click());
		if key_resp.clicked() {
			self.waiting_key = true;
		}

		let key_bg = if self.waiting_key { Color32::from_rgb(32, 34, 40) } else { CONTROL_BG };
		ui.painter().rect_filled(key_rect, 6.0, key_bg);
		ui.painter().rect_stroke(key_rect, 6.0, Stroke::new(1.0_f32, CONTROL_BORDER), StrokeKind::Inside);

		let key_text = if self.waiting_key { "Press...".into() } else { key_label(self.config.right_key) };
		let font_key = FontId::new(14.0, FontFamily::Name("Inter-SemiBold".into()));
		ui.painter().text(key_rect.center(), Align2::CENTER_CENTER, key_text, font_key, TEXT);

		label_semibold(ui, middle + 36.0, rect.top() + 26.0, "Items", 18.0, TEXT);
		let item_start = middle + 38.0;
		let total_width = rect.right() - item_start - 10.0;
		let item_spacing = total_width / 5.0;

		for (index, item) in ITEMS.iter().enumerate() {
			let x = item_start + index as f32 * item_spacing;
			checkbox(
				ui,
				Rect::from_min_size(Pos2::new(x, rect.top() + 56.0), egui::vec2(item_spacing - 6.0, 24.0)),
				item,
				&mut self.config.items[index],
			);
		}

		let author_font = FontId::new(13.0, FontFamily::Proportional);
		ui.painter().text(
			Pos2::new(rect.right() - 10.0, rect.top() + 108.0),
			Align2::RIGHT_CENTER,
			"Author: SimSool",
			author_font,
			Color32::from_rgb(115, 118, 128),
		);
	}
}

#[cfg(windows)]
impl eframe::App for App {
	fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
		self.scan_clients();
		self.poll();
		self.check_injection();
		ctx.request_repaint_after(Duration::from_millis(120));
		let before = serde_json::to_string(&self.config).unwrap_or_default();
		self.select_key(ctx);
		self.render_title_bar(ctx);
		egui::CentralPanel::default()
			.frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(38, 0)))
			.show(ctx, |ui| {
				self.render_process(ui);
				divider(ui);
				self.render_modules(ui);
				divider(ui);
				self.render_click_rate(ui);
				divider(ui);
				self.render_conditions(ui);
				if let Some(status) = self.status.as_ref().filter(|s| Some(s.pid) == self.selected) {
					if !status.error.is_empty() {
						ui.colored_label(MUTED, format!("Bridge: {}", status.error));
					}
				} else if !self.message.is_empty() {
					ui.colored_label(MUTED, &self.message);
				}
			});
		let after = serde_json::to_string(&self.config).unwrap_or_default();
		if before != after {
			if let Err(error) = lunar_shared::save_config(&self.config) {
				self.message = format!("Cannot save settings: {error}");
			}
		}
	}
}

#[cfg(windows)]
fn setup_fonts(ctx: &egui::Context) {
	let mut fonts = FontDefinitions::default();
	fonts.font_data.insert(
		"inter_regular".to_owned(),
		FontData::from_static(include_bytes!("../assets/Inter-Regular.ttf")).into(),
	);
	fonts.font_data.insert(
		"inter_semibold".to_owned(),
		FontData::from_static(include_bytes!("../assets/Inter-SemiBold.ttf")).into(),
	);
	fonts.font_data.insert(
		"inter_bold".to_owned(),
		FontData::from_static(include_bytes!("../assets/Inter-Bold.ttf")).into(),
	);

	if let Some(vec) = fonts.families.get_mut(&FontFamily::Proportional) {
		vec.insert(0, "inter_regular".to_owned());
	}
	fonts.families.insert(
		FontFamily::Name("Inter-SemiBold".into()),
		vec!["inter_semibold".to_owned(), "inter_regular".to_owned()],
	);
	fonts.families.insert(
		FontFamily::Name("Inter-Bold".into()),
		vec!["inter_bold".to_owned(), "inter_semibold".to_owned(), "inter_regular".to_owned()],
	);
	ctx.set_fonts(fonts);
}

#[cfg(windows)]
fn divider(ui: &mut egui::Ui) {
	let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
	ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0_f32, LINE));
}

#[cfg(windows)]
fn label_semibold(ui: &egui::Ui, x: f32, y: f32, text: &str, size: f32, color: Color32) {
	let font = FontId::new(size, FontFamily::Name("Inter-SemiBold".into()));
	ui.painter().text(Pos2::new(x, y), Align2::LEFT_CENTER, text, font, color);
}

#[cfg(windows)]
fn label_regular(ui: &egui::Ui, x: f32, y: f32, text: &str, size: f32, color: Color32) {
	let font = FontId::new(size, FontFamily::Proportional);
	ui.painter().text(Pos2::new(x, y), Align2::LEFT_CENTER, text, font, color);
}

#[cfg(windows)]
fn toggle(ui: &egui::Ui, rect: Rect, enabled: &mut bool, id: &str) {
	let response = ui.interact(rect, ui.id().with(id), Sense::click());
	if response.clicked() {
		*enabled = !*enabled;
	}
	let bg_color = if *enabled { Color32::WHITE } else { Color32::from_rgb(46, 48, 54) };
	ui.painter().rect_filled(rect, 12.0, bg_color);
	let knob_x = if *enabled { rect.right() - 13.0 } else { rect.left() + 13.0 };
	let knob_color = if *enabled { BG } else { Color32::WHITE };
	ui.painter().circle_filled(Pos2::new(knob_x, rect.center().y), 8.5, knob_color);
}

#[cfg(windows)]
fn checkbox(ui: &egui::Ui, rect: Rect, title: &str, checked: &mut bool) {
	let response = ui.interact(rect, ui.id().with(title), Sense::click());
	if response.clicked() {
		*checked = !*checked;
	}
	let square = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - 10.0), egui::vec2(20.0, 20.0));
	if *checked {
		ui.painter().rect_filled(square, 4.5, Color32::WHITE);
		let x = square.left();
		let y = square.top();
		ui.painter().line_segment([Pos2::new(x + 5.0, y + 10.5), Pos2::new(x + 9.0, y + 14.5)], Stroke::new(2.2_f32, BG));
		ui.painter().line_segment([Pos2::new(x + 9.0, y + 14.5), Pos2::new(x + 15.5, y + 6.0)], Stroke::new(2.2_f32, BG));
	} else {
		ui.painter().rect_filled(square, 4.5, CONTROL_BG);
		ui.painter().rect_stroke(square, 4.5, Stroke::new(1.4_f32, Color32::from_rgb(68, 71, 80)), StrokeKind::Inside);
	}
	label_regular(ui, rect.left() + 30.0, rect.center().y, title, 14.5, TEXT);
}


#[cfg(windows)]
fn draw_syringe(painter: &egui::Painter, center: Pos2, color: Color32) {
	let stroke_main = Stroke::new(1.6_f32, color);
	let stroke_thin = Stroke::new(1.2_f32, color);

	// Needle tip
	painter.line_segment(
		[Pos2::new(center.x - 9.0, center.y + 9.0), Pos2::new(center.x - 4.5, center.y + 4.5)],
		stroke_thin,
	);

	// Syringe body
	let barrel_dir = Vec2::new(1.0, -1.0).normalized();
	let perp = Vec2::new(1.0, 1.0).normalized();
	let b_start = center + barrel_dir * -4.0;
	let b_end = center + barrel_dir * 4.5;
	let half_w = 3.2;

	let p1 = b_start - perp * half_w;
	let p2 = b_start + perp * half_w;
	let p3 = b_end + perp * half_w;
	let p4 = b_end - perp * half_w;

	painter.line_segment([p1, p2], stroke_main);
	painter.line_segment([p2, p3], stroke_main);
	painter.line_segment([p3, p4], stroke_main);
	painter.line_segment([p4, p1], stroke_main);

	// Graduations
	let g1 = center + barrel_dir * -1.5;
	let g2 = center + barrel_dir * 1.5;
	painter.line_segment([g1, g1 + perp * 2.2], stroke_thin);
	painter.line_segment([g2, g2 + perp * 2.2], stroke_thin);

	// Barrel flange
	let f_start = b_end - perp * 4.8;
	let f_end = b_end + perp * 4.8;
	painter.line_segment([f_start, f_end], stroke_main);

	// Plunger rod and thumb rest
	let rod_end = b_end + barrel_dir * 4.2;
	painter.line_segment([b_end, rod_end], stroke_thin);
	let cap_start = rod_end - perp * 3.2;
	let cap_end = rod_end + perp * 3.2;
	painter.line_segment([cap_start, cap_end], stroke_main);
}

#[cfg(windows)]
fn vk_for_key(key: egui::Key) -> Option<u16> {
	let name = format!("{key:?}");
	if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() {
		return Some(name.as_bytes()[0] as u16);
	}
	if let Some(digit) = name.strip_prefix("Num") {
		if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() {
			return Some(digit.as_bytes()[0] as u16);
		}
	}
	if let Some(number) = name.strip_prefix('F').and_then(|s| s.parse::<u16>().ok()) {
		if (1..=12).contains(&number) {
			return Some(0x70 + number - 1);
		}
	}
	None
}

#[cfg(windows)]
fn key_label(vk: u16) -> String {
	if (b'A' as u16..=b'Z' as u16).contains(&vk) {
		return (vk as u8 as char).to_string();
	}
	if (b'0' as u16..=b'9' as u16).contains(&vk) {
		return (vk as u8 as char).to_string();
	}
	if (0x70..=0x7b).contains(&vk) {
		return format!("F{}", vk - 0x70 + 1);
	}
	"R".into()
}