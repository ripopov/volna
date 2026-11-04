//! Recorded signal values and user-selected formats in the waveform panel.

use std::path::Path;
use volna_core::Theme;
use volna_core::app::{App, Command};
use volna_core::geometry::{Modifiers, MouseButton, Point, Rect, point};
use volna_core::scene::{FontRole, MonoMeasure, Prim, TextMeasure};
use volna_core::testing::a;
use volna_core::trace::TraceId;
use volna_core::wave::PointerEvent;
use volna_core::wave::model::MenuAction;
use volna_core::workspace::Workspace;
use volna_trace::data::source::Lookup;
use volna_trace::session::OpenSpec;
use vtr::Recorder;

fn recording() -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new().suffix(".vtr").tempfile().unwrap();
    let mut writer = vtr::Writer::create(file.path()).unwrap();
    let sensor = writer
        .scope_with(
            vtr::SourceOptions::new("sensor", None),
            vtr::ScopeType::Module,
            "",
        )
        .unwrap();
    let signed = writer
        .signal(
            vtr::SourceOptions::new("temperature", Some(sensor)),
            vtr::SignalKind::bits(32, vtr::LogicStates::Four),
            vtr::VarType::Integer,
            vtr::Direction::Output,
        )
        .unwrap();
    let unsigned = writer
        .signal(
            vtr::SourceOptions::new("elapsed", Some(sensor)),
            vtr::SignalKind::bits(64, vtr::LogicStates::Four),
            vtr::VarType::Time,
            vtr::Direction::Output,
        )
        .unwrap();
    let logic = writer
        .signal(
            vtr::SourceOptions::new("data", Some(sensor)),
            vtr::SignalKind::bits(32, vtr::LogicStates::Four),
            vtr::VarType::Logic,
            vtr::Direction::Output,
        )
        .unwrap();
    let wide = writer
        .signal(
            vtr::SourceOptions::new("wide", Some(sensor)),
            vtr::SignalKind::bits(4097, vtr::LogicStates::Two),
            vtr::VarType::Logic,
            vtr::Direction::Output,
        )
        .unwrap();
    let mut packed = vec![0; 513];
    packed[0] = 0x34;
    packed[512] = 1;
    writer.emit_packed(wide, 2, &packed).unwrap();
    let large_integer = writer
        .signal(
            vtr::SourceOptions::new("large_integer", Some(sensor)),
            vtr::SignalKind::bits(1024, vtr::LogicStates::Two),
            vtr::VarType::Time,
            vtr::Direction::Output,
        )
        .unwrap();
    let mut packed = vec![0; 128];
    packed[127] = 0x80;
    writer.emit_packed(large_integer, 2, &packed).unwrap();
    let large_signed = writer
        .signal(
            vtr::SourceOptions::new("large_signed", Some(sensor)),
            vtr::SignalKind::bits(1024, vtr::LogicStates::Two),
            vtr::VarType::Integer,
            vtr::Direction::Output,
        )
        .unwrap();
    writer.emit_packed(large_signed, 2, &packed).unwrap();
    let real = writer
        .signal(
            vtr::SourceOptions::new("fraction", Some(sensor)),
            vtr::SignalKind::Real,
            vtr::VarType::Real,
            vtr::Direction::Output,
        )
        .unwrap();
    let text = writer
        .signal(
            vtr::SourceOptions::new("status", Some(sensor)),
            vtr::SignalKind::VarLen,
            vtr::VarType::String,
            vtr::Direction::Output,
        )
        .unwrap();
    writer.emit_real(real, -1_234_567.89).unwrap();
    writer.emit_bytes(text, b"waiting for input").unwrap();
    writer
        .emit_u64(signed, u64::from((-123_i32) as u32))
        .unwrap();
    writer.emit_u64(unsigned, u64::MAX).unwrap();
    writer.emit_u64(logic, 0xabcd_1234).unwrap();
    writer.advance_time(100).unwrap();
    writer.emit_real(real, 1.234_567e-300).unwrap();
    writer.emit_u64(signed, 42).unwrap();
    writer.emit_u64(unsigned, 1024).unwrap();
    writer.emit_packed(logic, 4, &[0xaa; 8]).unwrap();
    writer.advance_time(175).unwrap();
    writer.emit_u64(unsigned, 1_000_000_000).unwrap();
    writer
        .emit_u64(signed, u64::from((-123_456_789_i32) as u32))
        .unwrap();
    writer.emit_real(real, 9.999_999e99).unwrap();
    writer.advance_time(180).unwrap();
    writer.emit_real(real, f64::from_bits(1)).unwrap();
    writer.advance_time(185).unwrap();
    writer.emit_real(real, f64::MAX).unwrap();
    writer.advance_time(190).unwrap();
    writer.emit_real(real, -0.125).unwrap();
    writer.advance_time(200).unwrap();
    writer.close().unwrap();
    file
}

fn pump(app: &mut App) {
    loop {
        let requests = app.take_requests();
        if requests.is_empty() {
            return;
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
}

fn open(path: &Path) -> App {
    let mut app = App::new();
    app.handle(Command::Open(OpenSpec::Path(path.into())));
    pump(&mut app);
    app
}

fn add_signals(app: &mut App, names: &[&str]) {
    let hierarchy = app.doc.hierarchy(TraceId::A).unwrap();
    let vars = names
        .iter()
        .map(|name| {
            let Lookup::Found(var) = hierarchy.find_var(&["sensor", *name], None) else {
                panic!("missing signal {name}");
            };
            a(var)
        })
        .collect();
    app.handle(Command::AddVars(vars));
    pump(app);
    frame(app);
}

fn frame(app: &mut App) {
    let panel = app.panels.focused_id();
    let theme = Theme::one_dark();
    app.layout_panel(
        panel,
        Rect::from_xywh(0.0, 0.0, 1800.0, 600.0),
        &theme,
        1.0,
        &mut MonoMeasure,
    )
    .unwrap();
    app.render_panel(panel, &theme, &mut MonoMeasure);
}

fn press(app: &mut App, position: Point) {
    app.handle(Command::Pointer(
        app.panels.focused_id(),
        PointerEvent::Down {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        },
    ));
}

fn release(app: &mut App) {
    app.handle(Command::Pointer(app.panels.focused_id(), PointerEvent::Up));
}

fn widen_values(app: &mut App) {
    let layout = app.panels.focused_waves().unwrap().last_layout();
    let start = point(layout.values.right(), layout.values.top() + 12.0);
    press(app, start);
    app.handle(Command::Pointer(
        app.panels.focused_id(),
        PointerEvent::Move {
            position: point(start.x + 240.0, start.y),
        },
    ));
    release(app);
    frame(app);
}

fn place_cursor(app: &mut App, time: u64) {
    let waves = app.panels.focused_waves().unwrap();
    let layout = waves.last_layout();
    let x = waves
        .viewport(&app.doc)
        .x_of_tick(time, f64::from(layout.waves.width()));
    let position = point(layout.waves.left() + x as f32, layout.row_y(0) + 12.0);
    press(app, position);
    release(app);
}

fn assert_values(app: &mut App, expected: &[&str]) {
    frame(app);
    let layout = app.panels.focused_waves().unwrap().last_layout().clone();
    let panel = app.panels.focused_id();
    let scene = app.render_panel(panel, &Theme::one_dark(), &mut MonoMeasure);
    for (row, expected) in expected.iter().enumerate() {
        let cell = Rect::from_xywh(
            layout.values.left(),
            layout.row_y(row),
            layout.values.width(),
            layout.row_height(row),
        );
        let texts: Vec<_> = scene
            .prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::Text { origin, text, .. } if cell.contains(*origin) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            texts.contains(expected),
            "row {row}: expected {expected:?}, displayed {texts:?}"
        );
    }
}

#[test]
fn inspecting_recorded_signals_shows_decimal_integers_and_hex_logic() {
    let file = recording();
    let mut app = open(file.path());
    add_signals(&mut app, &["temperature", "elapsed", "data"]);
    widen_values(&mut app);

    place_cursor(&mut app, 25);
    assert_values(&mut app, &["-123", "18446744073709551615", "abcd1234"]);
    place_cursor(&mut app, 150);
    assert_values(&mut app, &["42", "1024", "xxxxxxxx"]);
}

#[test]
fn reopening_a_workspace_preserves_one_signals_hex_override() {
    let file = recording();
    let mut app = open(file.path());
    add_signals(&mut app, &["temperature", "elapsed"]);
    widen_values(&mut app);
    place_cursor(&mut app, 25);
    assert_values(&mut app, &["-123", "18446744073709551615"]);

    let layout = app.panels.focused_waves().unwrap().last_layout();
    let name = point(layout.names.left() + 60.0, layout.row_y(0) + 12.0);
    press(&mut app, name);
    release(&mut app);
    let badge = app.panels.focused_waves().unwrap().last_layout().badges[0].1;
    press(&mut app, point(badge.left() + 2.0, badge.top() + 2.0));
    release(&mut app);
    app.handle(Command::MenuSelect(
        app.panels.focused_id(),
        MenuAction::Format("hex".into()),
    ));
    assert_values(&mut app, &["ffffff85", "18446744073709551615"]);

    let saved = Workspace::capture(&app, volna_core::testing::paths("trace.vtr"), None)
        .unwrap()
        .to_bytes()
        .unwrap();
    let mut reopened = open(file.path());
    Workspace::parse(&saved)
        .unwrap()
        .prepare(
            &reopened,
            "file:///tmp/trace.vtr",
            "file:///tmp/trace.vtr.volna.json",
        )
        .unwrap()
        .commit(&mut reopened)
        .unwrap();
    pump(&mut reopened);
    frame(&mut reopened);
    place_cursor(&mut reopened, 25);
    assert_values(&mut reopened, &["ffffff85", "18446744073709551615"]);
    place_cursor(&mut reopened, 150);
    assert_values(&mut reopened, &["0000002a", "1024"]);
}

fn fit_value_characters(app: &mut App, row: usize, characters: usize) {
    frame(app);
    let theme = Theme::one_dark();
    let layout = app.panels.focused_waves().unwrap().last_layout();
    let right = layout
        .badges
        .iter()
        .find(|(i, _)| *i == row)
        .unwrap()
        .1
        .left();
    let available = right - layout.values.left() - 8.0 * theme.zoom;
    let char_width = MonoMeasure.text_width("0", FontRole::Mono, theme.mono_size);
    let start = point(layout.values.right(), layout.row_y(row) + 12.0);
    let end = point(
        start.x + (characters as f32 + 0.25) * char_width - available,
        start.y,
    );
    press(app, start);
    app.handle(Command::Pointer(
        app.panels.focused_id(),
        PointerEvent::Move { position: end },
    ));
    release(app);
    frame(app);
}

#[test]
fn narrowing_decimal_values_preserves_sign_scale_and_marks_rounding() {
    let file = recording();
    let mut app = open(file.path());
    add_signals(&mut app, &["fraction"]);
    place_cursor(&mut app, 25);
    fit_value_characters(&mut app, 0, 8);
    assert_values(&mut app, &["≈-1.23e6"]);
    place_cursor(&mut app, 150);
    assert_values(&mut app, &["≈1e-300"]);
    place_cursor(&mut app, 177);
    assert_values(&mut app, &["≈1e100"]);
    place_cursor(&mut app, 182);
    assert_values(&mut app, &["≈5e-324"]);
    place_cursor(&mut app, 187);
    assert_values(&mut app, &["≈1.8e308"]);
    place_cursor(&mut app, 192);
    fit_value_characters(&mut app, 0, 4);
    assert_values(&mut app, &["…"]);
    fit_value_characters(&mut app, 0, 8);
    assert_values(&mut app, &["-0.125"]);

    let mut app = open(file.path());
    add_signals(&mut app, &["elapsed"]);
    place_cursor(&mut app, 25);
    fit_value_characters(&mut app, 0, 8);
    assert_values(&mut app, &["≈1.84e19"]);
    widen_values(&mut app);
    assert_values(&mut app, &["18446744073709551615"]);
    place_cursor(&mut app, 177);
    fit_value_characters(&mut app, 0, 3);
    assert_values(&mut app, &["1e9"]);

    let mut app = open(file.path());
    add_signals(&mut app, &["temperature"]);
    place_cursor(&mut app, 177);
    fit_value_characters(&mut app, 0, 8);
    assert_values(&mut app, &["≈-1.23e8"]);
}

#[test]
fn overflowing_buses_keep_their_base_and_both_ends() {
    let file = recording();
    let mut app = open(file.path());
    add_signals(&mut app, &["data"]);
    place_cursor(&mut app, 25);
    fit_value_characters(&mut app, 0, 7);
    assert_values(&mut app, &["0xab…34"]);
    let mut wide_app = open(file.path());
    add_signals(&mut wide_app, &["wide"]);
    place_cursor(&mut wide_app, 25);
    fit_value_characters(&mut wide_app, 0, 7);
    assert_values(&mut wide_app, &["0x10…34"]);

    let layout = app.panels.focused_waves().unwrap().last_layout();
    let badge = layout.badges[0].1;
    press(&mut app, point(badge.left() + 2.0, badge.top() + 2.0));
    release(&mut app);
    app.handle(Command::MenuSelect(
        app.panels.focused_id(),
        MenuAction::Format("bin".into()),
    ));
    fit_value_characters(&mut app, 0, 7);
    assert_values(&mut app, &["0b10…00"]);
    fit_value_characters(&mut app, 0, 4);
    assert_values(&mut app, &["…"]);
}

#[test]
fn overflowing_text_keeps_its_beginning() {
    let file = recording();
    let mut app = open(file.path());
    add_signals(&mut app, &["status"]);
    place_cursor(&mut app, 25);
    fit_value_characters(&mut app, 0, 6);
    assert_values(&mut app, &["\"wait…"]);
    widen_values(&mut app);
    assert_values(&mut app, &["\"waiting for input\""]);
}

#[test]
fn narrowing_large_decimal_values_retains_sign_and_magnitude() {
    let file = recording();
    let mut app = open(file.path());
    add_signals(&mut app, &["large_integer"]);
    place_cursor(&mut app, 25);
    fit_value_characters(&mut app, 0, 9);
    assert_values(&mut app, &["≈8.99e307"]);
    let mut app = open(file.path());
    add_signals(&mut app, &["large_signed"]);
    place_cursor(&mut app, 25);
    fit_value_characters(&mut app, 0, 10);
    assert_values(&mut app, &["≈-8.99e307"]);
}
