//! The eye: how light worked out across a world (light/) shows on a
//! screen. That light runs from night's thousandths to lamps' hundreds
//! (1 lights a surface to its colour: light's Brightness); a screen's
//! white is 1.
//!
//! The voxels engine meters the luminance its rays meet (voxels.wgsl's
//! `meter`, a count by stops). The eye adapts to it over a second: to its
//! median in part (`adapting`), from a clear day's, so that a dark place
//! shows brighter but still dark, and a bright one darker; and it shows
//! all but its brightest hundredth below white. Each channel, times the
//! exposure, goes through Reinhard's curve (voxels.wgsl's `shown`): a view
//! whose light the screen holds, as a clear day's, shows as it is.

use web_time::Instant;

/** Bins the meter counts luminances in, each a share of SPAN stops from 2^LOWEST up: below in the first, above in the last. */
pub const BINS: usize = 64;
pub const LOWEST: f32 = -16.0;
pub const SPAN: f32 = 24.0;

/** The shares of what it sees below the eye's marks: the median it adapts to, and the light it shows white. */
const MARKS: [f32; 2] = [0.5, 0.99];
/** The marks of a clear day's view, metered, in stops. */
const DAY: [f32; 2] = [-3.75, -0.75];
/** Seconds the eye takes to adapt by all but a share 1/e of the way. */
const SECONDS: f32 = 1.0;
/** Stops from what it adapts to within which it has adapted: frames come till then. */
const ADAPTED: f32 = 0.05;

/** How the eye shows light: stops of exposure added to what it adapts to, and how far it adapts (0 not at all, 1 a dark place as bright as day). */
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Eye {
    pub exposure: f32,
    pub adapting: f32,
}

impl Eye {
    /**
     * The multiple of light it shows, having adapted to `marks` (Adapted),
     * and the light it shows white: the brightest it marked, or white if
     * that is darker.
     */
    pub fn shows(self, [median, brightest]: [f32; 2]) -> [f32; 2] {
        let exposure = (self.exposure - self.adapting * (median - DAY[0])).exp2();
        [exposure, (exposure * brightest.exp2()).max(1.0)]
    }

    /** The panel's sliders of it. */
    pub fn panel(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::Slider::new(&mut self.exposure, -8.0..=8.0).step_by(0.25).text("Exposure (stops)"));
        ui.add(egui::Slider::new(&mut self.adapting, 0.0..=1.0).text("Adapting"));
    }
}

impl Default for Eye {
    fn default() -> Eye {
        Eye { exposure: 0.0, adapting: 0.75 }
    }
}

/** The marks the eye adapted to, in stops, and those it adapts towards, as last metered; when it last adapted. */
pub struct Adapted {
    marks: [f32; 2],
    towards: [f32; 2],
    at: Instant,
}

/** Adapted to a clear day's view, as if it came from one. */
impl Default for Adapted {
    fn default() -> Adapted {
        Adapted { marks: DAY, towards: DAY, at: Instant::now() }
    }
}

impl Adapted {
    pub fn marks(&self) -> [f32; 2] {
        self.marks
    }

    /** Whether it is adapting still. */
    pub fn adapting(&self) -> bool {
        (0..2).any(|k| (self.marks[k] - self.towards[k]).abs() > ADAPTED)
    }

    /** What the meter counted (BINS), if anything, to adapt towards. */
    pub fn metered(&mut self, bins: &[u32]) {
        if let [Some(median), Some(brightest)] = MARKS.map(|share| below(bins, share)) {
            self.towards = [median, brightest];
        }
    }

    /** Adapted for the time since it last was, or all the way if `still`: a picture drawn till settled is seen adapted. */
    pub fn adapt(&mut self, still: bool) {
        let now = Instant::now();
        let left = if still { 0.0 } else { (-(now - self.at).as_secs_f32() / SECONDS).exp() };
        self.marks = std::array::from_fn(|k| self.towards[k] + (self.marks[k] - self.towards[k]) * left);
        self.at = now;
    }
}

/** The stops below which `share` of what `bins` counted is, within its bin as if its counts were spread evenly; None if nothing. */
fn below(bins: &[u32], share: f32) -> Option<f32> {
    let part = bins.iter().sum::<u32>() as f32 * share;
    let mut under = 0.0;
    for (bin, &n) in bins.iter().enumerate() {
        let n = n as f32;
        if under + n >= part && n > 0.0 {
            return Some(LOWEST + (bin as f32 + (part - under) / n) * SPAN / BINS as f32);
        }
        under += n;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_eye_marks_the_median_and_all_but_the_brightest_of_what_it_sees() {
        let mut bins = [0; BINS];
        assert_eq!(below(&bins, 0.5), None, "nothing metered");
        // The black of a cave, ground, sky, a lamp: the median at the top of the ground's bin, the lamp left out.
        (bins[0], bins[20], bins[40], bins[63]) = (10, 40, 49, 1);
        let top = |bin: f32| LOWEST + bin * SPAN / BINS as f32;
        assert_eq!(below(&bins, 0.5), Some(top(21.0)));
        assert_eq!(below(&bins, 0.99), Some(top(41.0)));
    }

    #[test]
    fn the_eye_shows_a_day_as_it_is_a_night_brighter_and_the_brightest_white() {
        let eye = Eye::default();
        assert_eq!(eye.shows(DAY), [1.0, 1.0], "a clear day as it is");
        let [night, _] = eye.shows([DAY[0] - 8.0, DAY[1]]);
        assert!(night > 1.0 && night < 256.0, "a night brighter, not as bright as day: {night}");
        assert_eq!(eye.shows([DAY[0], 3.0])[1], 8.0, "white the brightest seen");
    }
}
