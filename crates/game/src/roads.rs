//! Road widths. The roads themselves are drawn by the terrain shader from
//! `ground_mask`; these widths size that mask, the clearings in the
//! vegetation and the marks drawn along roads.

/// Half-widths in metres, by road role. Generous: at command altitude a real
/// 3 m lane is under a pixel, and the network has to stay legible while zoomed
/// out. Same map-symbol exaggeration as `people::FIGURE_SCALE`.
/// The same as the road grading's (`scenario::grade`): the level bench is
/// sized from them.
pub use scenario::grade::{DRIVABLE_HALF_W, TRACK_HALF_W};

/// How much wider the casing is than the surface it sits under. A dark edge is
/// what separates a road from bare limestone soil of a similar tone — without
/// it the network reads as a smudge from anywhere above a few hundred metres.
pub const CASING_M: f32 = 2.2;
