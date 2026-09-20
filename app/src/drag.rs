//! Reordering rows by dragging a grip, with a pointer-measured insertion slot.

use dioxus::prelude::*;

/// A reorder in progress: the row picked up, and where it would land.
#[derive(Clone, PartialEq)]
pub struct Drag {
    /// Position of the dragged row in the displayed order.
    pub from: usize,
    /// Midpoint of every row along the drag axis, in display order, as of pick-up.
    midpoints: Vec<f64>,
    slot: usize,
    horizontal: bool,
}

impl Drag {
    fn aim(&mut self, along: f64) {
        if let Some(slot) = slot_under(self.from, &self.midpoints, along) {
            self.slot = slot;
        }
    }

    /// The pointer's coordinate along the drag axis.
    fn along(&self, event: &PointerEvent) -> f64 {
        let point = event.data().client_coordinates();
        if self.horizontal { point.x } else { point.y }
    }

    /// Whether the rows have been measured, so a release means anything yet.
    fn aimed(&self) -> bool {
        !self.midpoints.is_empty()
    }

    /// Where the insertion line sits: the display index the row would land before.
    pub fn line_before(&self) -> usize {
        if self.slot >= self.from {
            self.slot + 1
        } else {
            self.slot
        }
    }
}

/// Marks where the insertion line goes. A shadow rather than a border, so showing it
/// does not shift the rows the pointer is being aimed at.
pub fn row_class(base: &str, line: Option<usize>, index: usize, count: usize) -> String {
    let mut class = base.to_owned();
    if line == Some(index) {
        class.push_str(" drop-above");
    }
    if line == Some(count) && index + 1 == count {
        class.push_str(" drop-below");
    }
    class
}

/// Slot in the order without the dragged row, or `None` before the rows are measured --
/// an empty set of midpoints would otherwise read as "above everything".
fn slot_under(from: usize, midpoints: &[f64], y: f64) -> Option<usize> {
    if midpoints.is_empty() {
        return None;
    }
    let insertion = midpoints.iter().filter(|midpoint| y > **midpoint).count();
    // Taking the dragged row out shifts everything after it down by one.
    Some(if insertion > from {
        insertion - 1
    } else {
        insertion
    })
}

/// Midpoints of the rows matching `selector`, measured in the webview.
async fn row_midpoints(selector: &str, horizontal: bool) -> Vec<f64> {
    let (start, size) = if horizontal {
        ("left", "width")
    } else {
        ("top", "height")
    };
    let script = format!(
        "dioxus.send(Array.from(document.querySelectorAll('{selector}')).map((row) => {{
            const box = row.getBoundingClientRect();
            return box.{start} + box.{size} / 2;
        }}));"
    );
    document::eval(&script).recv().await.unwrap_or_default()
}

fn aim_drag(mut drag: Signal<Option<Drag>>, event: &PointerEvent) {
    if let Some(mut active) = drag() {
        let along = active.along(event);
        active.aim(along);
        drag.set(Some(active));
    }
}

fn finish_drag(mut drag: Signal<Option<Drag>>, on_drop: EventHandler<usize>) {
    if let Some(active) = drag.take().filter(Drag::aimed) {
        on_drop.call(active.slot);
    }
}

/// Catches move and release while dragging with a mouse. Touch pointers stay captured by
/// the grip, but mouse pointers retarget to whatever is under the cursor, so without this
/// a release away from the grip is never seen.
#[component]
pub fn DragCatcher(drag: Signal<Option<Drag>>, on_drop: EventHandler<usize>) -> Element {
    rsx! {
        div {
            class: "drag-catcher",
            onpointermove: move |event: PointerEvent| aim_drag(drag, &event),
            onpointerup: move |_| finish_drag(drag, on_drop),
            onpointercancel: move |_| drag.set(None),
        }
    }
}

/// The handle a row is dragged by. Pointer events during a touch drag all go to the
/// element the gesture started on, so every handler belongs here rather than on the list.
#[component]
pub fn Grip(
    index: usize,
    selector: String,
    #[props(default)] horizontal: bool,
    drag: Signal<Option<Drag>>,
    on_drop: EventHandler<usize>,
) -> Element {
    let mut drag = drag;
    rsx! {
        button {
            class: "grip",
            title: "drag to reorder",
            onpointerdown: move |event: PointerEvent| {
                // Set synchronously, so a release that beats the measurement below still
                // finds a drag to cancel rather than silently doing nothing.
                let fresh = Drag {
                    from: index,
                    midpoints: Vec::new(),
                    slot: index,
                    horizontal,
                };
                let along = fresh.along(&event);
                drag.set(Some(fresh));
                let selector = selector.clone();
                spawn(async move {
                    let midpoints = row_midpoints(&selector, horizontal).await;
                    if let Some(mut active) = drag() {
                        active.midpoints = midpoints;
                        active.aim(along);
                        drag.set(Some(active));
                    }
                });
            },
            onpointermove: move |event: PointerEvent| aim_drag(drag, &event),
            onpointerup: move |_| finish_drag(drag, on_drop),
            onpointercancel: move |_| drag.set(None),
            "⠿"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Three rows 20px tall starting at y=0.
    const MIDPOINTS: [f64; 3] = [10.0, 30.0, 50.0];

    #[test]
    fn dragging_the_first_row_downwards_walks_through_every_slot() {
        assert_eq!(slot_under(0, &MIDPOINTS, 5.0), Some(0));
        assert_eq!(slot_under(0, &MIDPOINTS, 15.0), Some(0));
        assert_eq!(slot_under(0, &MIDPOINTS, 35.0), Some(1));
        assert_eq!(slot_under(0, &MIDPOINTS, 55.0), Some(2));
    }

    #[test]
    fn dragging_the_last_row_upwards_walks_through_every_slot() {
        assert_eq!(slot_under(2, &MIDPOINTS, 5.0), Some(0));
        assert_eq!(slot_under(2, &MIDPOINTS, 25.0), Some(1));
        assert_eq!(slot_under(2, &MIDPOINTS, 55.0), Some(2));
    }

    // Above its own midpoint and just below it are both "stay put".
    #[test]
    fn a_row_dropped_on_itself_keeps_its_slot() {
        assert_eq!(slot_under(1, &MIDPOINTS, 25.0), Some(1));
        assert_eq!(slot_under(1, &MIDPOINTS, 35.0), Some(1));
    }

    // A release before the rows are measured must cancel, not move to the top.
    #[test]
    fn an_unmeasured_drag_has_no_slot_and_does_not_fire() {
        assert_eq!(slot_under(1, &[], 25.0), None);

        let mut drag = Drag {
            from: 1,
            midpoints: Vec::new(),
            slot: 1,
            horizontal: false,
        };
        drag.aim(5.0);
        assert_eq!(drag.slot, 1);
        assert!(!drag.aimed());
    }

    #[test]
    fn the_line_sits_where_the_row_would_land() {
        let mut drag = Drag {
            from: 0,
            midpoints: MIDPOINTS.to_vec(),
            slot: 0,
            horizontal: false,
        };

        drag.aim(5.0);
        assert_eq!(drag.line_before(), 1);
        drag.aim(35.0);
        assert_eq!(drag.line_before(), 2);
        drag.aim(55.0);
        assert_eq!(drag.line_before(), 3);

        drag.from = 2;
        drag.aim(5.0);
        assert_eq!(drag.line_before(), 0);
    }
}
