//! Aola tool views. Files, networking and game calls belong to the controller.
use std::collections::BTreeSet;

use egui::{Button, ComboBox, DragValue, RichText, ScrollArea, TextEdit};

#[derive(Clone, Debug)]
pub struct Row {
    pub id: u64,
    pub time: String,
    pub connection: u64,
    pub command: String,
    pub extension_id: Option<u16>,
    pub parameters: String,
    pub summary: String,
    pub export: Option<String>,
}

pub struct View<'a> {
    pub send_rows: &'a [Row],
    pub receive_rows: &'a [Row],
    pub status: &'a str,
    pub running: bool,
    pub logging: bool,
    pub bridge_ready: bool,
    pub login_visible: bool,
}

#[derive(Debug)]
pub enum Action {
    Start,
    Stop,
    Validate,
    OpenEditor,
    SaveEditor,
    Export(String),
    Clear,
    Logging(bool),
    OpenLogs,
    Playback { fps: f64, quality: String, mute: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Send,
    Receive,
    Editor,
}

pub struct ToolsUi {
    pub editor: String,
    pub interval_ms: u64,
    pub groups: u32,
    pub group_pause_ms: u64,
    pub fps: f64,
    pub quality: String,
    pub mute: bool,
    open: bool,
    tab: Tab,
    filter: String,
    selected: BTreeSet<u64>,
    detail: Option<u64>,
    paused: bool,
    frozen_send: Vec<Row>,
    frozen_receive: Vec<Row>,
}

impl Default for ToolsUi {
    fn default() -> Self {
        Self {
            editor: String::new(),
            interval_ms: 200,
            groups: 1,
            group_pause_ms: 1000,
            fps: 50.0,
            quality: "high".into(),
            mute: false,
            open: false,
            tab: Tab::Send,
            filter: String::new(),
            selected: BTreeSet::new(),
            detail: None,
            paused: false,
            frozen_send: Vec::new(),
            frozen_receive: Vec::new(),
        }
    }
}

impl ToolsUi {
    /// Pass the root UI used by the existing menu bar. This reserves its 24 px.
    pub fn show(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, view: &View<'_>) -> Vec<Action> {
        let mut actions = Vec::new();
        if view.login_visible {
            self.open = false;
            egui::Popup::close_all(ctx);
            if view.running { actions.push(Action::Stop); }
        }
        egui::Panel::top("aola_tools_toolbar")
            .exact_size(crate::gui::MENU_HEIGHT as f32)
            .show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.strong("奥拉星");
                    if ui.add_enabled(!view.login_visible, Button::new("抓包 / 发包")).clicked() {
                        self.open = true;
                    }
                    ui.separator();
                    ui.add_enabled_ui(!view.login_visible, |ui| {
                        ui.menu_button("播放设置", |ui| self.playback(ui, &mut actions));
                    });
                    let mut logging = view.logging;
                    if ui.checkbox(&mut logging, "保留日志").changed() {
                        actions.push(Action::Logging(logging));
                    }
                    if ui.button("日志目录").clicked() {
                        actions.push(Action::OpenLogs);
                    }
                    if view.login_visible && ui.available_width() >= 155.0 {
                        ui.weak("完成网页登录后可用");
                    }
                    if view.running {
                        ui.separator();
                        if ui.button("停止发送").clicked() {
                            actions.push(Action::Stop);
                        }
                    }
                });
            });

        if !self.open {
            return actions;
        }
        let mut open = self.open;
        egui::Window::new("奥拉星工具")
            .id(egui::Id::new("aola_tools_window"))
            .open(&mut open)
            .default_size([820.0, 460.0])
            .min_width(460.0)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Send, "发送记录");
                    ui.selectable_value(&mut self.tab, Tab::Receive, "接收消息");
                    ui.selectable_value(&mut self.tab, Tab::Editor, "发包编辑器");
                    ui.separator();
                    ui.label(if view.bridge_ready { "发送桥已就绪" } else { "等待游戏发送桥" });
                });
                ui.separator();
                if self.tab == Tab::Editor {
                    self.show_editor(ui, view, &mut actions);
                } else {
                    self.show_records(ui, view, &mut actions);
                }
                ui.separator();
                ui.label(view.status);
            });
        if !open && view.running {
            actions.push(Action::Stop);
        }
        self.open = open;
        actions
    }

    fn playback(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label("目标帧率");
            ui.add(DragValue::new(&mut self.fps).range(1.0..=120.0).speed(1.0));
        });
        ComboBox::from_id_salt("aola_quality")
            .selected_text(quality_name(&self.quality))
            .show_ui(ui, |ui| {
                for (value, label) in [("low", "低"), ("medium", "中"), ("high", "高"), ("best", "最佳")] {
                    ui.selectable_value(&mut self.quality, value.to_owned(), label);
                }
            });
        ui.checkbox(&mut self.mute, "静音");
        if ui.button("应用并保存").clicked() {
            actions.push(Action::Playback {
                fps: self.fps,
                quality: self.quality.clone(),
                mute: self.mute,
            });
            ui.close();
        }
    }

    fn show_records(&mut self, ui: &mut egui::Ui, view: &View<'_>, actions: &mut Vec<Action>) {
        ui.horizontal_wrapped(|ui| {
            ui.label("筛选");
            ui.add(TextEdit::singleline(&mut self.filter).desired_width(190.0).hint_text("命令、扩展或参数"));
            if ui.checkbox(&mut self.paused, "暂停显示").changed() {
                if self.paused {
                    self.frozen_send = view.send_rows.to_vec();
                    self.frozen_receive = view.receive_rows.to_vec();
                } else {
                    self.frozen_send.clear();
                    self.frozen_receive.clear();
                }
            }
            if ui.button("清空记录").clicked() {
                self.frozen_send.clear();
                self.frozen_receive.clear();
                self.selected.clear();
                self.detail = None;
                actions.push(Action::Clear);
            }
        });
        if self.paused {
            ui.weak("显示已暂停；连接观察和已开启的日志继续运行。");
        }
        let send = self.tab == Tab::Send;
        let rows = match (self.paused, send) {
            (true, true) => self.frozen_send.as_slice(),
            (true, false) => self.frozen_receive.as_slice(),
            (false, true) => view.send_rows,
            (false, false) => view.receive_rows,
        };
        let filtered: Vec<&Row> = rows.iter().filter(|row| matches_filter(row, &self.filter)).collect();
        let mut append = None;
        if send {
            // Retiring a bounded record also retires its selection.
            let active_ids: BTreeSet<u64> = rows.iter().map(|row| row.id).collect();
            self.selected.retain(|id| active_ids.contains(id));
            ui.horizontal_wrapped(|ui| {
                if ui.button("选择筛选结果").clicked() {
                    self.selected.extend(filtered.iter().filter(|r| r.export.is_some()).map(|r| r.id));
                }
                if ui.button("取消选择").clicked() {
                    self.selected.clear();
                }
                let enabled = !self.selected.is_empty();
                if ui.add_enabled(enabled, Button::new("复制 #send")).clicked() {
                    ui.ctx().copy_text(selected_text(rows, &self.selected));
                }
                if ui.add_enabled(enabled, Button::new("导出文件")).clicked() {
                    actions.push(Action::Export(selected_text(rows, &self.selected)));
                }
                if ui.add_enabled(enabled && !view.running, Button::new("追加到编辑器")).clicked() {
                    append = Some(selected_text(rows, &self.selected));
                }
            });
        }
        ui.weak(format!("显示 {} / {} 条{}", filtered.len(), rows.len(), if send { " · 仅可导出的消息能勾选" } else { "" }));
        let height = (ui.available_height() - 125.0).clamp(90.0, 280.0);
        ScrollArea::vertical()
            .id_salt(if send { "aola_send_rows" } else { "aola_receive_rows" })
            .max_height(height)
            .auto_shrink([false, false])
            .show_rows(ui, 24.0, filtered.len(), |ui, range| {
                for index in range {
                    let row = filtered[index];
                    ui.push_id(row.id, |ui| {
                        ui.horizontal(|ui| {
                            if send {
                                let mut selected = self.selected.contains(&row.id);
                                if ui.add_enabled(row.export.is_some(), egui::Checkbox::without_text(&mut selected)).changed() {
                                    if selected { self.selected.insert(row.id); } else { self.selected.remove(&row.id); }
                                }
                            }
                            ui.monospace(&row.time);
                            ui.weak(format!("连接 {}", row.connection));
                            let label = match row.extension_id {
                                Some(id) => format!("{id} / {}", row.command),
                                None => row.command.clone(),
                            };
                            if ui.selectable_label(self.detail == Some(row.id), label).clicked() {
                                self.detail = Some(row.id);
                            }
                            ui.add(egui::Label::new(&row.summary).truncate()).on_hover_text(&row.summary);
                        });
                    });
                }
            });
        if let Some(row) = rows.iter().find(|row| Some(row.id) == self.detail) {
            ui.horizontal(|ui| {
                ui.label("消息详情");
                if ui.button("复制详情").clicked() {
                    ui.ctx().copy_text(row.parameters.clone());
                }
            });
            ScrollArea::vertical().id_salt("aola_packet_detail").max_height(125.0).show(ui, |ui| {
                let mut text = row.parameters.as_str();
                ui.add(TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(4));
            });
        } else {
            ui.weak("点击命令查看参数。");
        }
        if let Some(text) = append {
            if !self.editor.is_empty() && !self.editor.ends_with('\n') { self.editor.push('\n'); }
            self.editor.push_str(&text);
            self.tab = Tab::Editor;
        }
    }

    fn show_editor(&mut self, ui: &mut egui::Ui, view: &View<'_>, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            if ui.add_enabled(!view.running, Button::new("打开文件")).clicked() {
                actions.push(Action::OpenEditor);
            }
            if ui.button("保存文本").clicked() { actions.push(Action::SaveEditor); }
            if ui.add_enabled(!view.running, Button::new("检查指令")).clicked() {
                actions.push(Action::Validate);
            }
        });
        ui.weak("#send={\"id\":42,\"param\":{},\"cmd\":\"CMD\"}|   #time=1|   支持 // 整行注释");
        ui.add_enabled_ui(!view.running, |ui| {
            ScrollArea::vertical().id_salt("aola_send_editor").max_height(230.0).show(ui, |ui| {
                ui.add(TextEdit::multiline(&mut self.editor)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(10)
                    .hint_text("粘贴 #send 指令，或从发送记录追加"));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("消息间隔（毫秒）");
                ui.add(DragValue::new(&mut self.interval_ms).range(0..=60_000));
                ui.label("执行组数");
                ui.add(DragValue::new(&mut self.groups).range(1..=1000));
                ui.label("组间休息（毫秒）");
                ui.add(DragValue::new(&mut self.group_pause_ms).range(0..=3_600_000));
            });
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(!view.running && view.bridge_ready && !self.editor.trim().is_empty(), Button::new("开始发送")).clicked() {
                actions.push(Action::Start);
            }
            if ui.add_enabled(view.running, Button::new("停止")).clicked() {
                actions.push(Action::Stop);
            }
            ui.label(RichText::new("已提交给游戏 ≠ 服务器执行成功").small());
        });
    }
}

fn quality_name(value: &str) -> &'static str {
    match value { "low" => "低", "medium" => "中", "best" => "最佳", _ => "高" }
}

fn matches_filter(row: &Row, query: &str) -> bool {
    let query = query.trim();
    query.is_empty()
        || row.command.contains(query)
        || row.parameters.contains(query)
        || row.summary.contains(query)
        || row.extension_id.is_some_and(|id| id.to_string().contains(query))
}

fn selected_text(rows: &[Row], selected: &BTreeSet<u64>) -> String {
    rows.iter().filter(|row| selected.contains(&row.id)).filter_map(|row| row.export.as_deref()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aola_login_closes_overlays_and_stops_pending_sends() {
        let ctx = egui::Context::default();
        let mut tools = ToolsUi { open: true, ..Default::default() };
        let view = View { send_rows: &[], receive_rows: &[], status: "", running: true,
            logging: false, bridge_ready: true, login_visible: true };
        let mut actions = Vec::new();
        let _ = ctx.run_ui(Default::default(), |ui| actions = tools.show(&ctx, ui, &view));
        assert!(!tools.open);
        assert!(actions.iter().any(|action| matches!(action, Action::Stop)));

        // The offline preview has no login child and still permits the tool window.
        tools.open = true;
        let preview = View { running: false, login_visible: false, ..view };
        let _ = ctx.run_ui(Default::default(), |ui| actions = tools.show(&ctx, ui, &preview));
        assert!(tools.open);
        assert!(!actions.iter().any(|action| matches!(action, Action::Stop)));
    }
}
