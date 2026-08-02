//! The buffering indicator inside the play button.
//!
//! A spinner reads as "the application is busy". Waiting on a stream is a
//! different thing, so this draws a wave travelling across a row of bars —
//! closer to the visualiser the stream is about to feed, and calmer than a
//! rotating shape at this size.

use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::f64::consts::TAU;
use std::rc::Rc;

/// Bars in the wave.
const BARS: usize = 5;
/// Gap between bars, in pixels.
const GAP: f64 = 3.0;
/// Shortest and tallest a bar gets, as a fraction of the height.
const MIN_SCALE: f64 = 0.22;
const MAX_SCALE: f64 = 1.0;
/// Radians the wave advances per second.
const SPEED: f64 = 4.2;
/// Phase offset between neighbouring bars. Enough to read as a travelling
/// wave rather than bars pulsing in unison.
const SPREAD: f64 = 0.85;

pub struct BufferingIndicator {
    area: gtk::DrawingArea,
    phase: Rc<Cell<f64>>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

impl BufferingIndicator {
    pub fn new() -> Self {
        let phase = Rc::new(Cell::new(0.0));

        let area = gtk::DrawingArea::builder()
            .content_width(30)
            .content_height(26)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            // Decorative; the button's own label carries the meaning.
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();

        let drawn = phase.clone();
        area.set_draw_func(move |_, context, width, height| {
            draw(context, width as f64, height as f64, drawn.get());
        });

        Self {
            area,
            phase,
            tick: RefCell::new(None),
        }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// Starts or stops the animation. Idle when inactive, so a stopped player
    /// is not repainting sixty times a second.
    pub fn set_active(&self, active: bool) {
        if active == self.tick.borrow().is_some() {
            return;
        }

        match active {
            true => {
                let phase = self.phase.clone();
                let last = Cell::new(None::<i64>);

                let tick = self.area.add_tick_callback(move |area, clock| {
                    // Drive from the frame clock so the wave keeps its speed
                    // regardless of how often frames actually land.
                    let now = clock.frame_time();
                    let elapsed = match last.replace(Some(now)) {
                        Some(previous) => (now - previous) as f64 / 1_000_000.0,
                        None => 0.0,
                    };

                    phase.set((phase.get() + elapsed * SPEED) % TAU);
                    area.queue_draw();
                    glib::ControlFlow::Continue
                });
                *self.tick.borrow_mut() = Some(tick);
            }
            false => {
                if let Some(tick) = self.tick.borrow_mut().take() {
                    tick.remove();
                }
                self.phase.set(0.0);
                self.area.queue_draw();
            }
        }
    }
}

impl Default for BufferingIndicator {
    fn default() -> Self {
        Self::new()
    }
}

/// Height of bar `index` at `phase`, as a fraction of the full height.
///
/// Offsetting each bar along the same sine is what makes the row read as one
/// wave passing through rather than bars bouncing independently.
fn bar_scale(index: usize, phase: f64) -> f64 {
    let wave = (phase - index as f64 * SPREAD).sin();
    MIN_SCALE + (MAX_SCALE - MIN_SCALE) * (0.5 + 0.5 * wave)
}

fn draw(context: &gtk::cairo::Context, width: f64, height: f64, phase: f64) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    let count = BARS as f64;
    let bar_width = ((width - GAP * (count - 1.0)) / count).max(1.0);
    let radius = bar_width / 2.0;
    let centre = height / 2.0;

    context.set_source_rgba(1.0, 1.0, 1.0, 0.95);

    for index in 0..BARS {
        let bar_height = (height * bar_scale(index, phase)).max(bar_width);
        let x = index as f64 * (bar_width + GAP);
        let y = centre - bar_height / 2.0;

        rounded_bar(context, x, y, bar_width, bar_height, radius);
        let _ = context.fill();
    }
}

fn rounded_bar(
    context: &gtk::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let (right, bottom) = (x + width, y + height);
    let half_pi = std::f64::consts::FRAC_PI_2;

    context.new_sub_path();
    context.arc(right - radius, y + radius, radius, -half_pi, 0.0);
    context.arc(right - radius, bottom - radius, radius, 0.0, half_pi);
    context.arc(
        x + radius,
        bottom - radius,
        radius,
        half_pi,
        std::f64::consts::PI,
    );
    context.arc(
        x + radius,
        y + radius,
        radius,
        std::f64::consts::PI,
        1.5 * std::f64::consts::PI,
    );
    context.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_heights_stay_within_the_drawable_range() {
        for step in 0..64 {
            let phase = step as f64 * TAU / 64.0;
            for index in 0..BARS {
                let scale = bar_scale(index, phase);
                assert!(
                    (MIN_SCALE..=MAX_SCALE).contains(&scale),
                    "bar {index} at phase {phase} scaled to {scale}"
                );
            }
        }
    }

    #[test]
    fn neighbouring_bars_differ_so_it_reads_as_a_wave() {
        // Bars moving in unison would just be a pulse.
        let differences: Vec<f64> = (1..BARS)
            .map(|i| (bar_scale(i, 0.0) - bar_scale(i - 1, 0.0)).abs())
            .collect();
        assert!(
            differences.iter().any(|d| *d > 0.05),
            "bars are too close to in phase: {differences:?}"
        );
    }

    #[test]
    fn the_wave_repeats_over_a_full_turn() {
        for index in 0..BARS {
            let start = bar_scale(index, 0.0);
            let full_turn = bar_scale(index, TAU);
            assert!((start - full_turn).abs() < 1e-9);
        }
    }
}
