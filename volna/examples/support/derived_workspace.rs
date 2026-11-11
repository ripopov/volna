//! Curated views built through the workspace reader and the real computation pipeline.

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::Path;
use volna_core::{
    App,
    computed::{
        Definition, FormulaId, FormulaSettings, MeasurementScope, Operation, UnknownPolicy,
    },
    data::NumericKind,
    trace::{TraceId, Traced},
    workspace::Workspace,
};
use volna_trace::{data::source::Lookup, session::OpenSpec};

fn pump(app: &mut App) -> Result<()> {
    for _ in 0..100 {
        let requests = app.take_requests();
        if requests.is_empty() {
            return Ok(());
        }
        for request in requests {
            app.deliver(request.perform());
        }
    }
    anyhow::bail!("example computations did not settle")
}

struct Views<'a> {
    app: &'a App,
    next_formula: u64,
}

impl Views<'_> {
    fn recorded(&self, path: &str) -> Result<Value> {
        let input = self.input(
            path,
            path.rsplit('.').next().unwrap(),
            NumericKind::Unsigned,
        )?;
        let shape = input.shape;
        Ok(json!({
            "type": "signal", "shape": shape, "signal": path.split('.').collect::<Vec<_>>(),
            "computed": null, "projection": null,
            "bus": matches!(shape, volna_trace::data::SignalShape::Vector { .. }).then(|| json!({"collapsed": true, "initialized": false})),
            "format": if shape == volna_trace::data::SignalShape::Real { "real" } else if shape.width() == 1 { "bit" } else { "udec" },
        }))
    }

    fn input(
        &self,
        path: &str,
        alias: &str,
        kind: NumericKind,
    ) -> Result<volna_core::computed::Input> {
        let hierarchy = self
            .app
            .doc
            .hierarchy(TraceId::A)
            .context("missing hierarchy")?;
        let Lookup::Found(var) = hierarchy.find_var(&path.split('.').collect::<Vec<_>>(), None)
        else {
            anyhow::bail!("missing example signal {path}");
        };
        let mut input = self.app.recorded_input(Traced::new(TraceId::A, var))?;
        input.alias = alias.into();
        if input.shape != volna_trace::data::SignalShape::Real {
            input.interpretation.kind = kind;
        }
        Ok(input)
    }

    fn formula(
        &mut self,
        name: &str,
        expression: &str,
        bindings: &[(&str, &str)],
    ) -> Result<Value> {
        let inputs = bindings
            .iter()
            .map(|&(alias, path)| {
                self.input(
                    path,
                    alias,
                    if path == "sensors.adc_code" {
                        NumericKind::Signed
                    } else {
                        NumericKind::Unsigned
                    },
                )
            })
            .collect::<Result<Vec<_>>>()?;
        self.next_formula += 1;
        let definition = Definition {
            id: FormulaId(self.next_formula),
            revision: 0,
            name: name.into(),
            operation: Operation::Formula {
                expression: expression.into(),
                settings: FormulaSettings {
                    default_step: super::CLOCK_PS,
                    saved_range: self.app.doc.limits(),
                    scope: MeasurementScope::WholeRange,
                    unknown: UnknownPolicy::Gap,
                },
            },
        };
        let shape = definition
            .validate(&inputs)
            .with_context(|| name.to_owned())?;
        let saved_inputs = inputs
            .iter()
            .zip(bindings)
            .map(|(input, &(_, path))| {
                json!({
                    "source": "recorded", "alias": input.alias, "trace": "A",
                    "path": path.split('.').collect::<Vec<_>>(), "nth": null,
                    "shape": input.shape, "interpretation": input.interpretation,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "type": "signal", "computed": {"definition": definition, "inputs": saved_inputs},
            "shape": shape, "projection": null, "bus": null, "signal": [],
            "format": if shape.width() == 1 { "bit" } else { "real" },
            "tint": "cyan",
        }))
    }

    fn stream(&mut self) -> Result<Vec<Value>> {
        let clk = ("clk", "stream.clk");
        let reset = ("reset_n", "stream.reset_n");
        let valid = ("valid", "stream.valid");
        let ready = ("ready", "stream.ready");
        let payload = ("bytes", "stream.payload_bytes");
        let accepted = ("accepted", "stream.accepted_count");
        let completed = ("completed", "stream.completed_count");
        Ok(vec![
            group(
                "Ready/valid · 476 ps clock",
                vec![
                    self.recorded("stream.clk")?,
                    self.recorded("stream.reset_n")?,
                    self.recorded("stream.valid")?,
                    self.recorded("stream.ready")?,
                    self.recorded("stream.payload_bytes")?,
                    self.formula(
                        "Transfer · rising edge",
                        "(reset_n && valid && ready) @posedge clk",
                        &[reset, valid, ready, clk],
                    )?,
                    self.formula(
                        "Backpressure · any input change",
                        "reset_n && valid && !ready",
                        &[reset, valid, ready],
                    )?,
                ],
            ),
            group(
                "Queue · conservation of packets",
                vec![
                    analog(self.recorded("stream.queue_depth")?, 2),
                    analog(
                        self.formula(
                            "In flight [packets]",
                            "accepted - completed",
                            &[accepted, completed],
                        )?,
                        2,
                    ),
                    self.recorded("stream.accepted_count")?,
                    self.recorded("stream.completed_count")?,
                    self.recorded("stream.accepted_bytes")?,
                ],
            ),
            group(
                "Measurements · whole recording",
                vec![
                    self.formula(
                        "Transferred packets",
                        "interval_sum(real(reset_n && valid && ready) @posedge clk)",
                        &[reset, valid, ready, clk],
                    )?,
                    self.formula(
                        "Transferred bytes",
                        "interval_sum((real(reset_n && valid && ready) * bytes) @posedge clk)",
                        &[reset, valid, ready, payload, clk],
                    )?,
                    self.formula(
                        "Backpressured cycles",
                        "interval_sum(real(reset_n && valid && !ready) @posedge clk)",
                        &[reset, valid, ready, clk],
                    )?,
                    self.formula(
                        "Peak queue [packets]",
                        "interval_max(accepted - completed)",
                        &[accepted, completed],
                    )?,
                ],
            ),
        ])
    }

    fn sensors(&mut self) -> Result<Vec<Value>> {
        let voltage = ("v", "sensors.supply_v");
        let current = ("i", "sensors.load_a");
        let temperature = ("temp", "sensors.temperature_c");
        let adc = ("adc", "sensors.adc_code");
        let clk = ("clk", "stream.clk");
        Ok(vec![
            group(
                "Recorded sensors",
                vec![
                    analog(self.recorded("sensors.supply_v")?, 3),
                    analog(self.recorded("sensors.load_a")?, 3),
                    analog(self.recorded("sensors.temperature_c")?, 3),
                ],
            ),
            group(
                "Power · arithmetic, filtering, integration",
                vec![
                    analog(self.formula("Power [W]", "v * i", &[voltage, current])?, 3),
                    analog(
                        self.formula(
                            "Power · 20 ns mean [W]",
                            "moving_avg(v * i, 20ns, step: 476ps, warmup: \"available\")",
                            &[voltage, current],
                        )?,
                        3,
                    ),
                    analog(
                        self.formula(
                            "Energy [nJ]",
                            "integrate(v * i, initial: 0, step: 476ps) * 1000000000",
                            &[voltage, current],
                        )?,
                        3,
                    ),
                ],
            ),
            group(
                "Signed ADC · 1024 codes/V",
                vec![
                    {
                        let mut row = self.recorded("sensors.adc_code")?;
                        row["format"] = json!("sdec");
                        row
                    },
                    analog(self.formula("ADC voltage [V]", "adc / 1024", &[adc])?, 3),
                    self.formula("ADC limit exceeded", "abs(adc / 1024) > 1.25", &[adc])?,
                ],
            ),
            group(
                "Measurements · physical units",
                vec![
                    self.formula(
                        "Mean power [W]",
                        "interval_mean(v * i)",
                        &[voltage, current],
                    )?,
                    self.formula(
                        "Total energy [nJ]",
                        "interval_integral(v * i) * 1000000000",
                        &[voltage, current],
                    )?,
                    self.formula("Peak temperature [C]", "interval_max(temp)", &[temperature])?,
                    {
                        let mut row = self.formula(
                            "Mean power · clock sampled [W]",
                            "interval_mean((v * i) @posedge clk)",
                            &[voltage, current, clk],
                        )?;
                        row["computed"]["definition"]["operation"]["settings"]["scope"] =
                            json!(MeasurementScope::Explicit {
                                start: super::CLOCK_PS / 2,
                                end: self.app.doc.limits().1
                            });
                        row
                    },
                ],
            ),
        ])
    }

    fn statistics(&mut self) -> Result<Vec<Value>> {
        let rd = ("rd", "stats.lds_read_bytes");
        let wr = ("wr", "stats.lds_write_bytes");
        let conflict = ("conflict", "stats.lds_conflict_cycles");
        let accesses = ("accesses", "stats.lds_bank_accesses");
        let req = ("req", "stats.lds_requests");
        let sample = ("sample", "stats.sample_number");
        let clocks = ("clocks", "stats.clock_count");
        Ok(vec![
            group(
                "LDS counters · 100-clock windows",
                vec![
                    self.recorded("stats.sample_number")?,
                    self.recorded("stats.clock_count")?,
                    self.recorded("stats.lds_read_bytes")?,
                    self.recorded("stats.lds_write_bytes")?,
                    self.recorded("stats.lds_conflict_cycles")?,
                    self.recorded("stats.lds_bank_accesses")?,
                    self.recorded("stats.lds_requests")?,
                ],
            ),
            group(
                "Derived · sample-number changes",
                vec![
                    analog(
                        self.formula(
                            "LDS traffic [GB/s]",
                            "((rd + wr) / 47.6) @sample",
                            &[rd, wr, sample],
                        )?,
                        3,
                    ),
                    analog(
                        self.formula(
                            "LDS traffic [bytes/clock]",
                            "((rd + wr) / 100) @sample",
                            &[rd, wr, sample],
                        )?,
                        2,
                    ),
                    analog(
                        self.formula(
                            "Extra bank cycles/request",
                            "(conflict / req) @sample",
                            &[conflict, req, sample],
                        )?,
                        3,
                    ),
                    analog(
                        self.formula(
                            "Bank words/request",
                            "(accesses / req) @sample",
                            &[accesses, req, sample],
                        )?,
                        2,
                    ),
                    self.formula(
                        "Bank conflict present",
                        "(conflict > 0) @sample",
                        &[conflict, sample],
                    )?,
                ],
            ),
            group(
                "Measurements · whole excerpt",
                vec![
                    self.formula(
                        "Total LDS bytes",
                        "interval_sum((rd + wr) @sample)",
                        &[rd, wr, sample],
                    )?,
                    self.formula(
                        "Total LDS requests",
                        "interval_sum(req @sample)",
                        &[req, sample],
                    )?,
                    self.formula(
                        "Weighted extra cycles/request",
                        "interval_sum(conflict @sample) / interval_sum(req @sample)",
                        &[conflict, req, sample],
                    )?,
                    self.formula(
                        "Mean LDS traffic [bytes/clock]",
                        "interval_sum((rd + wr) @sample) / interval_max(clocks)",
                        &[rd, wr, sample, clocks],
                    )?,
                ],
            ),
        ])
    }
}

fn analog(mut row: Value, height: u8) -> Value {
    row["height"] = json!(height);
    row["analog"] = json!({"draw": "step", "range": "trace"});
    row
}

fn group(name: &str, rows: Vec<Value>) -> Value {
    json!({"type": "group", "name": name, "rows": rows})
}

pub fn open(path: &Path, bytes: &[u8]) -> Result<App> {
    let mut app = App::new();
    app.set_session(OpenSpec::Path(path.into()).open()?);
    let location = url::Url::from_file_path(std::fs::canonicalize(path)?)
        .map_err(|()| anyhow::anyhow!("trace path is not an absolute file path"))?;
    let plan = Workspace::parse(bytes)?.prepare(
        &app,
        location.as_str(),
        &format!("{location}.volna.example.json"),
    )?;
    ensure!(
        plan.report().notices.is_empty(),
        "workspace notices: {:?}",
        plan.report()
    );
    let report = plan.commit(&mut app)?;
    ensure!(report.notices.is_empty(), "restore notices: {report:?}");
    pump(&mut app)?;
    Ok(app)
}

pub fn write(path: &Path) -> Result<()> {
    let mut app = App::new();
    app.set_session(OpenSpec::Path(path.into()).open()?);
    let name = path
        .file_name()
        .context("missing trace filename")?
        .to_string_lossy()
        .into_owned();
    let Lookup::Found(clk) = app
        .doc
        .hierarchy(TraceId::A)
        .context("missing hierarchy")?
        .find_var(&["stream", "clk"], None)
    else {
        anyhow::bail!("missing stream clock");
    };
    app.handle(volna_core::Command::AddVars(vec![Traced::new(
        TraceId::A,
        clk,
    )]));
    let mut workspace = Workspace::capture(&app, |_| Ok(Some(name.clone())), None)?;
    let template = workspace
        .panels
        .iter()
        .find_map(|p| {
            let value: Value = serde_json::from_str(p.get()).ok()?;
            (value["kind"] == "waves").then_some(value)
        })
        .context("missing default wave panel")?;
    let mut views = Views {
        app: &app,
        next_formula: 0,
    };
    let view_rows = [
        (
            "Handshake & queue",
            views.stream()?,
            (0.0, (super::CLOCK_PS * 48) as f64),
        ),
        (
            "Power & sensors",
            views.sensors()?,
            (0.0, app.doc.limits().1 as f64),
        ),
        (
            "GPU statistics",
            views.statistics()?,
            (0.0, app.doc.limits().1 as f64),
        ),
    ];
    workspace.panels.clear();
    for (index, (title, rows, (start, end))) in view_rows.into_iter().enumerate() {
        let mut panel = template.clone();
        panel["id"] = json!(index + 1);
        panel["title"] = json!(title);
        panel["rows"] = json!(rows);
        panel["selected"] = json!([]);
        panel["link"]["viewport"] = json!(false);
        panel["link"]["cursor"] = json!(false);
        panel["viewport"] = json!({"start": start, "end": end});
        panel["cursor"] = json!(match index {
            0 => super::CLOCK_PS * 25 + super::CLOCK_PS / 2,
            1 => super::CLOCK_PS * 2000,
            _ => super::CLOCK_PS * super::SAMPLE_CLOCKS * 25,
        });
        panel["columns"] = json!({"names": 285.0, "scope": {"collapsed": true, "expanded_width": 100.0}, "values": 130.0});
        workspace
            .panels
            .push(serde_json::value::to_raw_value(&panel)?);
    }
    workspace.layout = serde_json::from_value(json!({
        "center": {"tabs": [1, 2, 3], "active": 1}, "right": null, "windows": {},
    }))?;
    workspace.focused = volna_core::panels::PanelId(1);
    workspace.shared.cursor = Some(super::CLOCK_PS * 25 + super::CLOCK_PS / 2);
    workspace.sidebar.selected_scope = Some(Traced::new(TraceId::A, vec!["stream".into()]));
    workspace.sidebar.width = 230.0;
    workspace.sidebar.scopes_fraction = 0.32;
    workspace.sampling = serde_json::from_value(json!({"choices": [
        {"trace": "A", "path": ["stream", "clk"], "nth": null, "mode": "rising_edge"},
        {"trace": "A", "path": ["stats", "sample_number"], "nth": null, "mode": "value_changes"},
    ]}))?;
    let mut restored = open(path, &workspace.to_bytes()?)?;
    for panel in restored.panels.iter() {
        if let Some(waves) = panel.kind.waves() {
            for row in waves.items().iter().filter_map(|entry| entry.signal()) {
                ensure!(row.error.is_none(), "{}: {:?}", row.name, row.error);
                ensure!(row.history.is_some(), "{} did not load", row.name);
                if let Some(volna_core::computed::MeasurementResult::Scalar(value)) =
                    row.measurement
                {
                    ensure!(
                        value.status == volna_core::computed::MeasurementStatus::Complete
                            && value.value.is_some(),
                        "{}: incomplete measurement {value:?}",
                        row.name
                    );
                }
            }
        }
    }
    let stats_panel = volna_core::panels::PanelId(3);
    let traffic = restored
        .panels
        .waves(stats_panel)
        .context("missing GPU statistics")?
        .items()
        .iter()
        .position(|entry| {
            entry
                .signal()
                .is_some_and(|row| row.name == "LDS traffic [GB/s]")
        })
        .context("missing bandwidth waveform")?;
    restored.handle(volna_core::Command::Derived(
        stats_panel,
        volna_core::computed::Command::Open(Some(traffic)),
    ));
    pump(&mut restored)?;
    restored.handle(volna_core::Command::Derived(
        stats_panel,
        volna_core::computed::Command::ApplyTable,
    ));
    pump(&mut restored)?;
    let table = restored
        .panels
        .iter()
        .find(|panel| panel.kind.table().is_some())
        .context("bandwidth table did not open")?
        .id;
    restored
        .panels
        .rename(table, Some("Bandwidth samples [GB/s]".into()))?;
    let mut canonical = Workspace::capture(&restored, |_| Ok(Some(name.clone())), None)?;
    canonical.layout = serde_json::from_value(
        json!({"center": {"tabs": [1, 2, 3, table], "active": 1}, "right": null, "windows": {}}),
    )?;
    canonical.focused = volna_core::panels::PanelId(1);
    open(path, &canonical.to_bytes()?)?;
    let output = path.with_file_name(format!("{name}.volna.example.json"));
    std::fs::write(&output, canonical.to_bytes()?)?;
    println!(
        "{}: {} curated views",
        output.display(),
        canonical.panels.len()
    );
    Ok(())
}
