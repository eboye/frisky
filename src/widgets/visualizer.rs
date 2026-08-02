//! A live audio visualiser drawn over the cover art.
//!
//! Decibels draws the waveform of a *file*: it decodes the whole thing up front,
//! so it can show the future as well as the past. Live radio has neither — there
//! is no file and no way to know what has not been broadcast yet. So this shows
//! the audio as it actually arrives: amplitudes from GStreamer's `level`
//! element, scrolling right to left, newest at the right.
//!
//! It fades out on hover so the artwork underneath is never permanently
//! obscured.

use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

/// Minimum bar height so silence still reads as a centre line.
const MIN_BAR_HEIGHT: f64 = 2.0;

/// How much of the previous value each new sample keeps. Without this the
/// waveform jitters; too much and it turns to mush.
const SMOOTHING: f64 = 0.35;
/// Per-frame decay applied once audio stops, so the waveform sinks away
/// instead of freezing.
const DECAY: f64 = 0.88;

/// How fast the observed range grows to take in a new extreme. High, so a
/// sudden peak is never clipped.
const RANGE_ATTACK: f64 = 0.4;
/// How fast the range closes back in once the extremes stop arriving. Low, so
/// the scale does not lurch between beats.
const RANGE_RELEASE: f64 = 0.0025;
/// Narrowest the observed range may get, in normalised units (~5 dB).
///
/// Without a floor on the span, a steady tone would be stretched into wild
/// swings driven by nothing but rounding.
const MIN_RANGE_SPAN: f64 = 0.08;
/// Below this level the input is treated as silence and left unstretched, so
/// the noise floor between tracks does not get amplified into a light show.
const SILENCE_LEVEL: f64 = 0.04;
/// How much of the final value comes from the auto-ranged signal rather than
/// the raw one. Keeping some raw signal means a quiet passage still looks
/// quieter than a loud one.
const RANGE_BLEND: f64 = 0.75;

/// Tracks the range of levels actually being received and rescales into it.
///
/// Broadcast audio is heavily limited: RMS sits in a narrow band near the top,
/// so a fixed dBFS scale draws a nearly flat wall. Following the observed
/// minimum and maximum turns those small variations into visible movement, the
/// way an auto-ranging meter does.
#[derive(Debug)]
struct AutoRange {
    floor: f64,
    ceiling: f64,
}

impl Default for AutoRange {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoRange {
    fn new() -> Self {
        // Start wide and let the signal close it in; starting narrow would make
        // the first seconds swing wildly.
        Self {
            floor: 0.0,
            ceiling: 1.0,
        }
    }

    /// Forgets the learned range, for when a new stream starts.
    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Rescales `level` into the range observed so far.
    fn normalise(&mut self, level: f64) -> f64 {
        let level = level.clamp(0.0, 1.0);

        // Track extremes: jump out to meet them, ease back in without them.
        let ceiling_rate = if level > self.ceiling {
            RANGE_ATTACK
        } else {
            RANGE_RELEASE
        };
        self.ceiling += (level - self.ceiling) * ceiling_rate;

        let floor_rate = if level < self.floor {
            RANGE_ATTACK
        } else {
            RANGE_RELEASE
        };
        self.floor += (level - self.floor) * floor_rate;

        // Near-silence should read as near-silence, not as amplified hiss.
        if self.ceiling < SILENCE_LEVEL {
            return level;
        }

        // Widen a collapsed range about its midpoint before dividing by it.
        let mut floor = self.floor;
        let mut ceiling = self.ceiling;
        let span = ceiling - floor;
        if span < MIN_RANGE_SPAN {
            let midpoint = (ceiling + floor) / 2.0;
            floor = midpoint - MIN_RANGE_SPAN / 2.0;
            ceiling = midpoint + MIN_RANGE_SPAN / 2.0;
        }

        let scaled = ((level - floor) / (ceiling - floor)).clamp(0.0, 1.0);
        (scaled * RANGE_BLEND + level * (1.0 - RANGE_BLEND)).clamp(0.0, 1.0)
    }
}

/// Size and density of a visualiser, so the same widget can serve both the
/// cover overlay and the compact bar.
#[derive(Debug, Clone, Copy)]
pub struct VisualizerSize {
    /// Bars on screen. At one sample per 50 ms, 64 bars holds about three
    /// seconds of history.
    pub bars: usize,
    pub width: i32,
    pub height: i32,
    pub gap: f64,
    /// Fraction of the height the loudest bar may occupy.
    pub scale: f64,
    /// Whether to paint the darkening scrim behind the bars. Needed over
    /// artwork, pointless on a solid gradient.
    pub scrim: bool,
}

impl VisualizerSize {
    /// Fills the cover art.
    pub const COVER: Self = Self {
        bars: 64,
        width: 300,
        height: 300,
        gap: 3.0,
        scale: 0.68,
        scrim: true,
    };

    /// Spans the whole mini player as a faint backdrop layer. Drawn at low
    /// opacity in CSS, so it carries no scrim of its own — the gradient tint
    /// underneath already handles legibility.
    pub const COMPACT_BACKDROP: Self = Self {
        bars: 48,
        width: 400,
        height: 76,
        gap: 3.0,
        scale: 0.92,
        scrim: false,
    };
}

pub struct Visualizer {
    size: VisualizerSize,
    area: gtk::DrawingArea,
    bars: Rc<RefCell<VecDeque<f64>>>,
    /// Last value pushed, used to smooth the next one.
    previous: Rc<Cell<f64>>,
    range: Rc<RefCell<AutoRange>>,
}

impl Visualizer {
    pub fn new(size: VisualizerSize) -> Self {
        let bars: Rc<RefCell<VecDeque<f64>>> =
            Rc::new(RefCell::new(VecDeque::from(vec![0.0; size.bars])));

        let area = gtk::DrawingArea::builder()
            .content_width(size.width)
            .content_height(size.height)
            // Purely decorative, and the information is available as text
            // elsewhere in the window.
            .can_target(false)
            // Purely decorative, so keep it out of the accessibility tree.
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        area.add_css_class("visualizer");

        let drawn = bars.clone();
        area.set_draw_func(move |_, context, width, height| {
            draw(context, width as f64, height as f64, &drawn.borrow(), size);
        });

        Self {
            size,
            area,
            bars,
            previous: Rc::new(Cell::new(0.0)),
            range: Rc::new(RefCell::new(AutoRange::new())),
        }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// Feeds in a new amplitude on a 0.0..=1.0 scale.
    pub fn push(&self, level: f64) {
        let level = self.range.borrow_mut().normalise(level);
        let smoothed = self.previous.get() * SMOOTHING + level * (1.0 - SMOOTHING);
        self.previous.set(smoothed);

        {
            let mut bars = self.bars.borrow_mut();
            bars.push_back(smoothed);
            while bars.len() > self.size.bars {
                bars.pop_front();
            }
        }
        self.area.queue_draw();
    }

    /// Sinks the waveform towards silence. Returns whether anything is still
    /// moving, so the caller can stop ticking once it has settled.
    pub fn decay(&self) -> bool {
        let mut active = false;
        {
            let mut bars = self.bars.borrow_mut();
            for bar in bars.iter_mut() {
                *bar *= DECAY;
                if *bar > 0.001 {
                    active = true;
                }
            }
        }
        self.previous.set(self.previous.get() * DECAY);
        self.area.queue_draw();
        active
    }

    /// Forgets the learned level range, so a new stream is measured afresh
    /// rather than against the last one.
    pub fn reset_range(&self) {
        self.range.borrow_mut().reset();
    }

    /// Fades the visualiser out, e.g. while the pointer is over the artwork.
    pub fn set_faded(&self, faded: bool) {
        if faded {
            self.area.add_css_class("faded");
        } else {
            self.area.remove_css_class("faded");
        }
    }
}

/// Draws the bars, mirrored about the vertical centre.
fn draw(
    context: &gtk::cairo::Context,
    width: f64,
    height: f64,
    bars: &VecDeque<f64>,
    size: VisualizerSize,
) {
    if width <= 0.0 || height <= 0.0 || bars.is_empty() {
        return;
    }

    // A scrim keeps white bars legible over pale artwork. It fades with the
    // widget, so hovering reveals the art cleanly.
    if size.scrim {
        let scrim = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, height);
        scrim.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
        scrim.add_color_stop_rgba(0.5, 0.0, 0.0, 0.0, 0.34);
        scrim.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
        if context.set_source(&scrim).is_ok() {
            let _ = context.paint();
        }
    }

    let count = bars.len() as f64;
    let bar_width = ((width - size.gap * (count - 1.0)) / count).max(1.0);
    let radius = (bar_width / 2.0).min(3.0);
    let centre = height / 2.0;

    for (index, level) in bars.iter().enumerate() {
        let bar_height = (level * height * size.scale).max(MIN_BAR_HEIGHT);
        let x = index as f64 * (bar_width + size.gap);
        let y = centre - bar_height / 2.0;

        // Older samples sit further left and are drawn fainter, giving the
        // waveform a sense of direction.
        let age = index as f64 / count;
        let alpha = 0.35 + 0.55 * age;

        rounded_rect(context, x, y, bar_width, bar_height, radius);
        context.set_source_rgba(1.0, 1.0, 1.0, alpha);
        let _ = context.fill();
    }
}

fn rounded_rect(
    context: &gtk::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    if radius <= 0.0 {
        context.rectangle(x, y, width, height);
        return;
    }

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

    /// Runs `samples` through the range and returns the outputs.
    fn run(samples: &[f64]) -> Vec<f64> {
        let mut range = AutoRange::new();
        samples.iter().map(|s| range.normalise(*s)).collect()
    }

    /// Feeds a value long enough for the range to settle around it.
    fn settle(range: &mut AutoRange, level: f64, iterations: usize) -> f64 {
        let mut last = 0.0;
        for _ in 0..iterations {
            last = range.normalise(level);
        }
        last
    }

    #[test]
    fn narrow_loud_input_is_stretched_into_visible_movement() {
        // Exactly the broadcast case: everything between 0.80 and 0.88, which
        // on a fixed scale is a flat wall near the top.
        let mut range = AutoRange::new();
        for _ in 0..400 {
            for level in [0.80, 0.84, 0.88, 0.82] {
                range.normalise(level);
            }
        }

        let low = range.normalise(0.80);
        let high = range.normalise(0.88);

        assert!(
            high - low > 0.25,
            "expected the narrow band to be stretched, got {low}..{high}"
        );
    }

    #[test]
    fn a_constant_level_does_not_peg_the_display() {
        // A steady tone must not sit at full height forever; the range closes
        // in and the minimum span keeps it mid-scale.
        let mut range = AutoRange::new();
        let settled = settle(&mut range, 0.9, 4000);
        assert!(
            (0.2..=0.95).contains(&settled),
            "constant input settled at {settled}"
        );
    }

    #[test]
    fn silence_stays_silent() {
        // Below the silence threshold the range must not amplify the noise
        // floor into movement.
        let mut range = AutoRange::new();
        let settled = settle(&mut range, 0.0, 4000);
        assert!(settled < 0.05, "silence read as {settled}");
    }

    #[test]
    fn sudden_peaks_are_never_clipped() {
        let mut range = AutoRange::new();
        settle(&mut range, 0.3, 2000);

        // A transient far above the settled range still fits on screen.
        let peak = range.normalise(1.0);
        assert!(peak <= 1.0, "peak overflowed: {peak}");
        assert!(peak > 0.8, "peak was flattened: {peak}");
    }

    #[test]
    fn output_always_stays_in_range() {
        let samples: Vec<f64> = (0..500)
            .map(|i| ((i as f64) / 37.0).sin().abs())
            .chain([0.0, 1.0, 0.5])
            .collect();

        for value in run(&samples) {
            assert!(
                (0.0..=1.0).contains(&value),
                "auto-range produced {value}, outside 0..=1"
            );
        }
    }

    #[test]
    fn resetting_forgets_the_learned_range() {
        let mut range = AutoRange::new();
        settle(&mut range, 0.85, 3000);
        range.reset();

        assert_eq!(range.floor, 0.0);
        assert_eq!(range.ceiling, 1.0);
    }
}
