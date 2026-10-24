//! **The picture's transcendental maths** (doc 21 cut G.3b): sine, cosine,
//! exponential, logarithm and hypotenuse through [`libm`], so a frame is the
//! same bytes wherever it is drawn.
//!
//! `f32::sin` and its kin call the platform's own maths library — the C
//! runtime's on Windows, glibc's on the box, Rust's in the browser's wasm —
//! and those differ in the last place. The raycaster turns a heading into a
//! ray with a sine and a cosine, so a last-place difference moves a wall edge
//! by a pixel: the TUI on the box would not draw what a fixture blessed on
//! Windows says, and the browser would not draw what the TUI does. libm is
//! one pure-Rust implementation, the same on every target. `sqrt`, `floor`,
//! `round` and the four operations are IEEE-exact everywhere already and stay
//! as they are.
//!
//! A test holds the rest of the crate to it: no `.sin()`, `.cos()`,
//! `.sin_cos()`, `.tan()`, `.atan2(`, `.exp()`, `.ln()`, `.powf(`, `.powi(`
//! or `.hypot(` outside this file.

/// The deterministic forms, as methods on `f32` and `f64`; bring them in with
/// `use crate::math::Det as _;`.
pub trait Det: Sized {
    /// The sine (radians).
    fn det_sin(self) -> Self;
    /// The cosine (radians).
    fn det_cos(self) -> Self;
    /// The sine and the cosine (radians).
    fn det_sin_cos(self) -> (Self, Self);
    /// `e` to the power of `self`.
    fn det_exp(self) -> Self;
    /// The natural logarithm.
    fn det_ln(self) -> Self;
    /// `sqrt(self² + other²)` without the overflow.
    fn det_hypot(self, other: Self) -> Self;
}

impl Det for f32 {
    fn det_sin(self) -> f32 {
        libm::sinf(self)
    }
    fn det_cos(self) -> f32 {
        libm::cosf(self)
    }
    fn det_sin_cos(self) -> (f32, f32) {
        libm::sincosf(self)
    }
    fn det_exp(self) -> f32 {
        libm::expf(self)
    }
    fn det_ln(self) -> f32 {
        libm::logf(self)
    }
    fn det_hypot(self, other: f32) -> f32 {
        libm::hypotf(self, other)
    }
}

impl Det for f64 {
    fn det_sin(self) -> f64 {
        libm::sin(self)
    }
    fn det_cos(self) -> f64 {
        libm::cos(self)
    }
    fn det_sin_cos(self) -> (f64, f64) {
        libm::sincos(self)
    }
    fn det_exp(self) -> f64 {
        libm::exp(self)
    }
    fn det_ln(self) -> f64 {
        libm::log(self)
    }
    fn det_hypot(self, other: f64) -> f64 {
        libm::hypot(self, other)
    }
}

#[cfg(test)]
mod tests {
    use super::Det;

    /// The forms agree with the platform's to a few last places (they are
    /// the same functions, computed one way).
    #[test]
    fn the_forms_are_the_functions() {
        for x in [0.0f64, 0.5, 1.0, 2.0, -3.0, 10.0] {
            assert!((x.det_sin() - x.sin()).abs() < 1e-12);
            assert!((x.det_cos() - x.cos()).abs() < 1e-12);
            assert!((x.det_exp() - x.exp()).abs() < 1e-9 * x.exp().max(1.0));
            assert!((x.det_hypot(2.0) - x.hypot(2.0)).abs() < 1e-12);
            if x > 0.0 {
                assert!((x.det_ln() - x.ln()).abs() < 1e-12);
            }
            let (s, c) = (x as f32).det_sin_cos();
            assert!((s - (x as f32).sin()).abs() < 1e-5 && (c - (x as f32).cos()).abs() < 1e-5);
        }
    }

    /// **Nothing else in the crate calls the platform's maths** (G.3b): a
    /// frame drawn in the browser is the frame drawn on the box.
    #[test]
    fn no_other_file_calls_the_platforms_maths() {
        const BANNED: &[&str] = &[
            ".sin()",
            ".cos()",
            ".sin_cos()",
            ".tan()",
            ".atan2(",
            ".exp()",
            ".ln()",
            ".powf(",
            ".powi(",
            ".hypot(",
            ".asin()",
            ".acos()",
            ".atan()",
            ".log(",
            ".log2()",
            ".log10()",
            ".cbrt()",
        ];
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        for entry in std::fs::read_dir(&src).expect("src").flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == "math.rs")
                || path.extension().is_none_or(|e| e != "rs")
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if let Some(b) = BANNED.iter().find(|b| code.contains(*b)) {
                    offenders.push(format!("{}:{}: {b}", path.display(), n + 1));
                }
            }
        }
        assert!(offenders.is_empty(), "use crate::math::Det: {offenders:#?}");
    }
}
