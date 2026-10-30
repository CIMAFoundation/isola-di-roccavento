//! Road widths. The roads themselves are drawn by the terrain shader from
//! `ground_mask`; these widths size that mask, the clearings in the
//! vegetation and the marks drawn along roads.

/// Half-widths in metres, by road role. Generous: at command altitude a real
/// 3 m lane is under a pixel, and the network has to stay legible while zoomed
/// out. Same map-symbol exaggeration as `people::FIGURE_SCALE`.
pub const DRIVABLE_HALF_W: f32 = 4.8;
pub const TRACK_HALF_W: f32 = 1.6;

/// How much wider the casing is than the surface it sits under. A dark edge is
/// what separates a road from bare limestone soil of a similar tone — without
/// it the network reads as a smudge from anywhere above a few hundred metres.
pub const CASING_M: f32 = 2.2;
