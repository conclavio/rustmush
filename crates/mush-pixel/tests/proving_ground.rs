//! The heightfield producer on the proving ground's real relief (doc 21 cut
//! G.7a): the shipped pack decodes to the documented relief, and a frame of
//! it renders inside the palette, with the ground visibly not flat.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use mush_pixel::Frame;
use mush_pixel::heightfield::render;
use mush_pixel::raycast::{Camera, Ground};

use support::atlas::proving_ground;

/// Doc 16 §11 (16b cut X.6): the proving ground's relief runs from -400 to
/// +400 cm, with The Bluff authored on top of it since cut AE.1; the walls
/// and woods of its classes are there too.
#[test]
fn the_shipped_relief_decodes() {
    let t = proving_ground();
    let (w, h) = (t.map().width(), t.map().height());
    assert_eq!((w, h), (256, 256));
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    let mut walls = 0;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let e = t.elev_cm(x, y);
            lo = lo.min(e);
            hi = hi.max(e);
            if t.map().get(x, y) == Ground::Wall {
                walls += 1;
            }
        }
    }
    assert!(lo < 0 && hi > 0, "relief both ways: {lo}..{hi} cm");
    assert!(
        hi - lo >= 400,
        "at least four metres of relief: {lo}..{hi} cm"
    );
    assert!(walls > 0, "the classes carry walls");
}

/// A frame from the middle of the ground, at four headings: inside the
/// palette, and the ground's skyline is not one straight row (it would be
/// on flat ground).
#[test]
fn a_frame_of_it_renders_with_relief() {
    let t = proving_ground();
    for step in 0..4u32 {
        let cam = Camera {
            x_cm: 128_000,
            y_cm: 128_000,
            heading_mdeg: step * 90_000,
        };
        let mut f = Frame::new(640, 480);
        render(&mut f, &t, &cam, None, &[]);
        assert!(f.count_colours(256) <= 256, "heading {}", step * 90);
        // Each column's first ground row from the top (sky and haze are
        // blue-grey, ground is not): on flat ground every column would meet
        // the haze at the same row.
        let rows: Vec<usize> = (0..640)
            .step_by(16)
            .map(|x| {
                (0..480)
                    .find(|&y| {
                        let [r, g, b] = f.get(x, y);
                        !(b > r && b >= g)
                    })
                    .unwrap_or(480)
            })
            .collect();
        let (min, max) = (rows.iter().min().unwrap(), rows.iter().max().unwrap());
        assert!(
            max > min,
            "heading {}: a ragged skyline, {min}..{max}",
            step * 90
        );
    }
}
