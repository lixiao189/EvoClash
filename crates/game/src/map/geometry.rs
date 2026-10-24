//! Segment intersection tests used by shots and line of sight.

use super::*;

/// Slab test: entry fraction of segment `a -> b` into `rect`, if any.
pub fn segment_rect(a: Vec2, b: Vec2, rect: Rect) -> Option<f32> {
    let d = b - a;
    let mut t_min: f32 = 0.0;
    let mut t_max: f32 = 1.0;
    for axis in 0..2 {
        let (o, dir, lo, hi) = if axis == 0 {
            (a.x, d.x, rect.min.x, rect.max.x)
        } else {
            (a.y, d.y, rect.min.y, rect.max.y)
        };
        if dir.abs() < 1e-8 {
            if o < lo || o > hi {
                return None;
            }
        } else {
            let mut t1 = (lo - o) / dir;
            let mut t2 = (hi - o) / dir;
            if t1 > t2 {
                core::mem::swap(&mut t1, &mut t2);
            }
            t_min = t_min.max(t1);
            t_max = t_max.min(t2);
            if t_min > t_max {
                return None;
            }
        }
    }
    Some(t_min)
}

/// Entry fraction of segment `a -> b` into a circle, if any.
pub fn segment_circle(a: Vec2, b: Vec2, center: Vec2, radius: f32) -> Option<f32> {
    let d = b - a;
    let f = a - center;
    let c = f.length_squared() - radius * radius;
    if c <= 0.0 {
        return Some(0.0);
    }
    let a_coef = d.length_squared();
    if a_coef < 1e-8 {
        return None;
    }
    let b_coef = 2.0 * f.dot(d);
    let disc = b_coef * b_coef - 4.0 * a_coef * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b_coef - disc.sqrt()) / (2.0 * a_coef);
    (0.0..=1.0).contains(&t).then_some(t)
}
