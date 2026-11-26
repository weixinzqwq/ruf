//! Correlate synchronous switchPet work with the movie render and window present.
use std::sync::OnceLock;

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AOLA_PET_SWITCH_TIMING").as_deref() == Ok("1"))
}

pub struct SwitchRender {
    pub id: u64,
    pub logical_frame: u64,
    pub cpu_ns: u64,
    pub relation: &'static str,
}

pub struct SwitchEvent {
    pub id: u64,
    pub logical_frame: u64,
    pub trigger: &'static str,
    pub tick_stage: Option<&'static str>,
    pub phases_ns: [Option<u64>; 4],
    pub completed: bool,
    pub render: Option<SwitchRender>,
    pub present_id: Option<u64>,
    pub present_gap_ns: Option<u64>,
    pub status: &'static str,
}

#[derive(Default)]
pub(crate) struct SwitchFrames {
    pub(crate) in_render: bool,
    next_id: u64,
    awaiting_render: Vec<SwitchEvent>,
    awaiting_present: Vec<SwitchEvent>,
    ready: Vec<SwitchEvent>,
}

impl SwitchFrames {
    pub(crate) fn record(&mut self, frame: u64, in_frame: bool, tick_stage: Option<&'static str>, phases_ns: [Option<u64>; 4], completed: bool) {
        self.next_id += 1;
        self.awaiting_render.push(SwitchEvent {
            id: self.next_id, logical_frame: frame, tick_stage,
            trigger: if self.in_render { "renderCallback" } else if in_frame { "logicalFrame" } else { "betweenFrames" },
            phases_ns, completed, render: None, present_id: None, present_gap_ns: None, status: "notRendered",
        });
    }

    pub(crate) fn rendered(&mut self, frame: u64, id: u64, cpu_ns: u64) {
        self.in_render = false;
        // GUI composition samples only the latest movie target. Earlier renders
        // that never reached a successful present cannot inherit its present ID.
        for mut event in self.awaiting_present.drain(..) {
            event.status = "overwrittenBeforePresent";
            self.ready.push(event);
        }
        for mut event in self.awaiting_render.drain(..) {
            let relation = match event.trigger {
                "renderCallback" => "duringRender",
                "logicalFrame" if event.logical_frame == frame => "sameLogicalFrame",
                "logicalFrame" => "logicalFramesCoalesced",
                _ => "nextRenderAfterCallback",
            };
            event.render = Some(SwitchRender { id, logical_frame: frame, cpu_ns, relation });
            event.status = "notPresented";
            self.awaiting_present.push(event);
        }
    }

    pub(crate) fn presented(&mut self, id: u64, gap_ns: Option<u64>) {
        for mut event in self.awaiting_present.drain(..) {
            event.present_id = Some(id);
            event.present_gap_ns = gap_ns;
            event.status = "presented";
            self.ready.push(event);
        }
    }

    pub(crate) fn has_events(&self) -> bool {
        !(self.awaiting_render.is_empty() && self.awaiting_present.is_empty() && self.ready.is_empty())
    }

    pub(crate) fn take(&mut self, end_of_movie: bool) -> Vec<SwitchEvent> {
        if end_of_movie {
            self.ready.append(&mut self.awaiting_present);
            self.ready.append(&mut self.awaiting_render);
        }
        std::mem::take(&mut self.ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pet_switch_frame_correlation_survives_separate_renders_and_presents() {
        let mut collector = SwitchFrames::default();
        let phases = [Some(1), Some(2), Some(3), Some(4)];
        collector.record(8, true, None, phases, true);
        collector.record(9, true, None, phases, true);
        collector.presented(40, None); // Only the previous movie image was visible.
        collector.rendered(9, 12, 8500);
        assert!(collector.take(false).is_empty());
        collector.presented(41, Some(15000));
        collector.presented(42, Some(20000)); // Redraw of the same image.
        let events = collector.take(false);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].render.as_ref().unwrap().relation, "logicalFramesCoalesced");
        assert_eq!(events[1].render.as_ref().unwrap().relation, "sameLogicalFrame");
        for event in events {
            assert_eq!(event.present_id, Some(41));
            assert_eq!(event.present_gap_ns, Some(15000));
            let render = event.render.unwrap();
            assert_eq!((render.id, render.logical_frame, render.cpu_ns), (12, 9, 8500));
        }
    }

    #[test]
    fn pet_switch_frame_end_keeps_unpresented_and_unrendered_distinct() {
        let mut collector = SwitchFrames::default();
        collector.record(8, false, Some("sockets"), [None; 4], false);
        collector.rendered(8, 12, 8500);
        collector.in_render = true;
        collector.record(8, false, None, [Some(1); 4], true);
        collector.rendered(8, 13, 6100);
        collector.record(8, false, None, [Some(2); 4], true);
        let events = collector.take(true);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].status, "overwrittenBeforePresent");
        assert_eq!(events[0].tick_stage, Some("sockets"));
        assert_eq!(events[0].render.as_ref().unwrap().relation, "nextRenderAfterCallback");
        assert_eq!(events[1].status, "notPresented");
        assert_eq!(events[1].render.as_ref().unwrap().relation, "duringRender");
        assert_eq!(events[2].status, "notRendered");
        assert!(events.iter().all(|event| event.present_id.is_none()));
        assert!(!collector.has_events());
    }
}
