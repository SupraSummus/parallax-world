//! The sites a layer reads for: their spacing, which a field or a thing
//! asks to leave out what is finer than its sites, and where in their
//! voxels' footprint across it a line of them reads what is dithered
//! (plans in the ground crate). Set by the engine as it draws; read by
//! whatever draws.

use std::cell::Cell;

thread_local! {
    /**
     * The sites a layer reads for: their spacing, and how far from where
     * it asks it reads dithered plans finer than that (`footprint`);
     * none while plans are made and points read their chances (`where_asked`).
     */
    static FOOTPRINT: Cell<Option<(i64, [i64; 3])>> = const { Cell::new(None) };
}

/**
 * Runs `read`, a layer's reads for sites `spacing` apart, with dithered
 * plans finer than the spacing read `offset` from where asked, none of
 * it along the line of sites reading: how the line reads them at one
 * point of its voxels' footprint across it (see the ground crate's
 * Plan::dithered).
 */
pub fn footprint<R>(spacing: i64, offset: [i64; 3], read: impl FnOnce() -> R) -> R {
    debug_assert!((spacing as u64).is_power_of_two(), "sites are a power of two apart");
    let outer = FOOTPRINT.replace(Some((spacing, offset)));
    let value = read();
    FOOTPRINT.set(outer);
    value
}

/** The sites read for now (`footprint`): their spacing and the offset dithered plans are read at; none where asked. */
#[inline]
pub fn read_for() -> Option<(i64, [i64; 3])> {
    FOOTPRINT.get()
}

/** The spacing of the sites a layer reads for now (`footprint`); 1 where asked, as plans and points are made. */
#[inline]
pub fn spacing() -> i64 {
    FOOTPRINT.get().map_or(1, |(spacing, _)| spacing)
}

/**
 * Where lines of sites `spacing` apart read, lattice units on from
 * `spacing` · site: the level-0 site under their cubes' middle, so that a
 * coarser site stands for the finer ones in its cube (an octree), and
 * level 1 reads one of its eight. Level 0 reads its cube [site, site + 1)
 * at its least corner, so the middle of what it reads in a coarser cube
 * is half a unit under the cube's: a coarser level is half a unit lower
 * still, which matters less the coarser it is. Shapes, in fixed point,
 * read each cube at its middle (draw_shape).
 */
pub fn middle(spacing: i64) -> i64 {
    (spacing - 1) / 2
}

/** Runs `read` with plans read where asked, whoever asks: as a plan's columns are made, and points' chances (points.rs). */
pub fn where_asked<R>(read: impl FnOnce() -> R) -> R {
    let outer = FOOTPRINT.take();
    let value = read();
    FOOTPRINT.set(outer);
    value
}
