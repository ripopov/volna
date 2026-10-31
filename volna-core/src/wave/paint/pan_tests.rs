//! Pan invariants for digital traces and dense bus state.

use super::tests::bus;
use super::*;
use crate::scene::{MonoMeasure, Prim};
use volna_trace::data::history::VecHistory;

fn assert_pan_stable(h: &dyn SignalHistory, start: f64, span: f64) {
    let t = Theme::volna(true);
    let area = Rect::from_xywh(37.25, 0.0, 400.5, 24.0);
    let translators = crate::data::Translators::builtin();
    let translator = translators.get("hex").unwrap();
    let draw = |pan: f64| {
        let vp = Viewport {
            start: start + pan * span / f64::from(area.width()),
            end: start + pan * span / f64::from(area.width()) + span,
        };
        let base = Viewport {
            start,
            end: start + span,
        };
        for period in [
            marks::HATCH_PX,
            marks::DOTTED.0 + marks::DOTTED.1,
            marks::DASHED.0 + marks::DASHED.1,
        ] {
            let delta = (pattern_origin(&vp, area, period) + pan as f32
                - pattern_origin(&base, area, period))
            .rem_euclid(period);
            assert!(delta.min(period - delta) < 0.001);
        }
        let mut scene = Scene::default();
        if h.shape() == SignalShape::Bit {
            paint_bit_row(h, &vp, area, &t, &mut scene);
        } else {
            let mut text = TextCache::default();
            let mut measure = MonoMeasure;
            let mut p = TextPainter {
                theme: &t,
                text: &mut text,
                measure: &mut measure,
                scene: &mut scene,
            };
            paint_bus_row(h, translator.as_ref(), &vp, area, area, 8.0, &mut p);
        }
        assert!(scene.prims.iter().all(|p| !matches!(p, Prim::Quad { .. })));
        let mut shapes = Vec::new();
        let inside = |a: f32, b: f32| a + pan as f32 > 100.01 && b + (pan as f32) < 349.99;
        for prim in scene.prims {
            match prim {
                Prim::SubpixelFill { rect, color: fill } => {
                    let left = (rect.left() + pan as f32).max(100.01);
                    let right = (rect.right() + pan as f32).min(349.99);
                    if right - left > 0.001 {
                        shapes.push((left, rect.top(), right, rect.bottom(), fill));
                    }
                }
                // Hatch lines overdraw their band clips; their source phase is checked above.
                Prim::Lines {
                    segments, color, ..
                } if color != t.wave_undef.with_alpha(marks::HATCH_ALPHA) => {
                    for [a, b] in segments {
                        if inside(a.x.min(b.x), a.x.max(b.x))
                            && ((a.x - b.x).abs() > 0.001 || (a.y - b.y).abs() > 0.001)
                        {
                            shapes.push((a.x + pan as f32, a.y, b.x + pan as f32, b.y, color));
                        }
                    }
                }
                _ => {}
            }
        }
        shapes
    };
    let baseline = draw(0.0);
    assert!(!baseline.is_empty(), "no geometry at {start} over {span}");

    for pan in [-12.9, -1.0, -0.01, 0.01, 0.3, 0.99, 1.0, 12.9] {
        let shifted = draw(pan);
        assert_eq!(baseline.len(), shifted.len(), "span {span}, pan {pan}");
        for (a, b) in baseline.iter().zip(&shifted) {
            assert_eq!(a.4, b.4, "span {span}, pan {pan}");
            for (x, y) in [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)] {
                assert!(
                    (x - y).abs() < 0.001,
                    "span {span}, pan {pan}: {a:?} != {b:?}"
                );
            }
        }
    }
}

#[test]
fn digital_geometry_translates_without_changing_during_fractional_pans() {
    let mut h = bus(&(0..2000)
        .map(|i| {
            (
                i * 7 + (i / 20) * 250,
                if i % 2 == 0 { "10000001" } else { "00000000" },
            )
        })
        .collect::<Vec<_>>());
    let mut bit = VecHistory {
        shape: SignalShape::Bit,
        times: h.times.clone(),
        values: (0..2000)
            .map(|i| WaveValue::Bits(if i % 2 == 0 { "1" } else { "0" }.into()))
            .collect(),
        initial: WaveValue::Bits("0".into()),
    };
    for span in [400.5, 801.0, 1602.0, 4005.0, 12345.67] {
        assert_pan_stable(&h, 1000.0, span);
        assert_pan_stable(&bit, 1000.0, span);
    }
    for state in ["x", "z", "-", "l"] {
        bit.values = (0..2000)
            .map(|i| WaveValue::Bits(if i % 2 == 0 { state } else { "0" }.into()))
            .collect();
        assert_pan_stable(&bit, 1000.0, 4005.0);
    }
    for state in ["xxxxxxxx", "zzzzzzzz", "10xz0101"] {
        h.values = (0..2000)
            .map(|i| WaveValue::Bits(if i % 2 == 0 { state } else { "00000000" }.into()))
            .collect();
        assert_pan_stable(&h, 1000.0, 4005.0);
    }
}

#[test]
fn dense_bus_style_is_independent_of_the_first_visible_value() {
    let t = Theme::volna(true);
    let translators = crate::data::Translators::builtin();
    let translator = translators.get("hex").unwrap();
    let area = Rect::from_xywh(37.25, 0.0, 400.5, 24.0);
    for spacing in [2, 32] {
        for unknown in [true, false] {
            let h = bus(&(0..3000)
                .map(|i| {
                    let bits = if (i * spacing >= 100) == unknown {
                        if i % 2 == 0 { "xxxxxxxx" } else { "uuuuuuuu" }
                    } else if i % 2 == 0 {
                        "00000000"
                    } else {
                        "11111111"
                    };
                    (i * spacing, bits)
                })
                .collect::<Vec<_>>());
            for start in [200.0, 71.0, 190.0, 210.0, 329.0] {
                let vp = Viewport {
                    start,
                    end: start + 4005.0,
                };
                let mut scene = Scene::default();
                let mut text = TextCache::default();
                let mut measure = MonoMeasure;
                let mut p = TextPainter {
                    theme: &t,
                    text: &mut text,
                    measure: &mut measure,
                    scene: &mut scene,
                };
                paint_bus_row(&h, translator.as_ref(), &vp, area, area, 8.0, &mut p);
                for tick in (1000..3000).step_by(16) {
                    let x = area.left() + vp.x_of_tick(tick, f64::from(area.width())) as f32;
                    let fills: Vec<_> = scene
                        .quads()
                        .filter(|(r, _)| {
                            r.left() <= x && r.right() > x && r.top() <= 12.0 && r.bottom() > 12.0
                        })
                        .map(|(_, c)| c)
                        .collect();
                    assert_eq!(fills.len(), 1);
                    if unknown {
                        assert_eq!(
                            fills[0], t.wave_undef_fill,
                            "start {start}, spacing {spacing}, tick {tick}"
                        );
                    } else {
                        assert_eq!(fills[0].with_alpha(1.0), t.wave_signal);
                        assert!(
                            scene.quads().all(|(r, c)| !(c == t.wave_undef
                                && r.left() <= x
                                && r.right() > x)),
                            "spurious unknown rail at start {start}, tick {tick}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn picorv32_axi_signals_are_stable_during_panning() {
    let session = volna_trace::session::OpenSpec::Path(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../volna/examples/picorv32.vtr"),
    )
    .open()
    .unwrap();
    for name in [
        "mem_axi_arready",
        "mem_axi_awready",
        "mem_axi_bvalid",
        "mem_axi_rdata",
        "trace_data",
    ] {
        let var = session
            .hierarchy()
            .vars()
            .find(|v| v.name.starts_with(name))
            .unwrap();
        let history = session.load_signal(var.signal).unwrap();
        for start in [2_500_000.0, 5_000_000.0] {
            assert_pan_stable(history.as_ref(), start, 4_005_000.0);
        }
    }
}
