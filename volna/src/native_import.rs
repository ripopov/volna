//! Native imports run through the installed converter before entering the VTR loader.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use volna_core::session::{LoadRequest, LoadResult};
use volna_core::trace::TraceId;
use volna_trace::session::{LocalSession, OpenSpec, Session};

#[derive(Default)]
pub(crate) struct Imports {
    pending: BTreeMap<(TraceId, u64), Cancellation>,
}

impl Imports {
    pub fn retain_current(&mut self, doc: &volna_core::document::Document) {
        self.pending.retain(|&(trace, generation), _| {
            doc.traces()
                .get(trace)
                .is_some_and(|slot| slot.generation() == generation)
        });
    }

    pub fn converting(&self, trace: TraceId) -> bool {
        self.pending.keys().any(|&(id, _)| id == trace)
    }

    pub fn finish(&mut self, result: &LoadResult) {
        if let LoadResult::Opened {
            trace, generation, ..
        } = result
        {
            self.pending.remove(&(*trace, *generation));
        }
    }

    pub fn prepare(
        &mut self,
        request: LoadRequest,
    ) -> impl FnOnce() -> LoadResult + Send + 'static + use<> {
        let cancel = match &request {
            LoadRequest::Open {
                trace,
                generation,
                spec: OpenSpec::Path(path),
            } if is_import(path) => {
                let cancellation = Cancellation::default();
                let flag = cancellation.0.clone();
                self.pending.insert((*trace, *generation), cancellation);
                Some(flag)
            }
            _ => None,
        };
        move || match (request, cancel) {
            (
                LoadRequest::Open {
                    trace,
                    generation,
                    spec: OpenSpec::Path(path),
                },
                Some(cancel),
            ) => LoadResult::Opened {
                trace,
                generation,
                result: open(&path, &cancel, converter_path),
            },
            (request, _) => request.perform(),
        }
    }
}

#[derive(Default)]
struct Cancellation(Arc<AtomicBool>);

impl Drop for Cancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn is_import(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let name = name.strip_suffix(".gz").unwrap_or(&name);
    [".fst", ".kanata", ".log"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

fn converter_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("VOLNA_VTR") {
        ensure!(!path.is_empty(), "VOLNA_VTR must name the vtr executable");
        return Ok(path.into());
    }
    let executable = std::env::current_exe().context("locate Volna executable")?;
    resolve_converter(&executable, std::env::var_os("PATH").as_deref())
}

fn resolve_converter(executable: &Path, search_path: Option<&std::ffi::OsStr>) -> Result<PathBuf> {
    let directory = executable
        .parent()
        .context("locate Volna installation directory")?;
    let tool = format!("vtr{}", std::env::consts::EXE_SUFFIX);
    let sibling = directory.join(&tool);
    if sibling.is_file() {
        return Ok(sibling);
    }
    if let Some(path) = search_path {
        for directory in std::env::split_paths(path) {
            let candidate = directory.join(&tool);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!(
        "Cannot find the vtr converter. Install vtr beside Volna or on PATH, or set VOLNA_VTR to its executable. For development, run `cargo build -p vtr-cli --bin vtr`."
    )
}

fn open(
    path: &Path,
    cancel: &AtomicBool,
    converter: impl FnOnce() -> Result<PathBuf>,
) -> Result<Arc<dyn Session>> {
    let mut header = Vec::with_capacity(8);
    std::fs::File::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .take(8)
        .read_to_end(&mut header)?;
    if header.starts_with(&vtr::container::FILE_MAGIC) {
        return OpenSpec::Path(path.into()).open();
    }
    ensure!(!cancel.load(Ordering::Relaxed), "conversion cancelled");
    let converter = converter()?;
    let directory = tempfile::Builder::new()
        .prefix("volna-import-")
        .tempdir()
        .context("create temporary conversion directory")?;
    let output = directory.path().join("recording.vtr");
    let mut diagnostics = tempfile::tempfile().context("create converter diagnostics file")?;
    let mut command = Command::new(&converter);
    command
        .arg("convert")
        .arg("--")
        .arg(path)
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(diagnostics.try_clone()?);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW keeps imports inside Volna's window.
    }
    let mut child = ConverterChild(command.spawn().with_context(|| format!(
        "Cannot run converter {}. Install the matching vtr executable beside Volna, or set VOLNA_VTR.", converter.display()
    ))?);
    let status = loop {
        ensure!(!cancel.load(Ordering::Relaxed), "conversion cancelled");
        if let Some(status) = child.0.try_wait().context("wait for vtr converter")? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        let length = diagnostics.metadata()?.len();
        diagnostics.seek(SeekFrom::Start(length.saturating_sub(16 * 1024)))?;
        let mut bytes = Vec::new();
        diagnostics.take(16 * 1024).read_to_end(&mut bytes)?;
        bail!(
            "Cannot convert {} to VTR ({status}).\n{}",
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy(),
            String::from_utf8_lossy(&bytes).trim()
        );
    }
    ensure!(!cancel.load(Ordering::Relaxed), "conversion cancelled");
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let session = LocalSession::open_temporary(&output, name, directory)
        .context("vtr converter did not produce a readable VTR recording")?;
    Ok(Arc::new(session))
}

struct ConverterChild(Child);

impl Drop for ConverterChild {
    fn drop(&mut self) {
        // Child does not kill or reap on drop, including early cancellation and I/O errors.
        drop(self.0.kill());
        drop(self.0.wait());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../vtr-convert/tests/fixtures")
            .join(name)
    }

    #[test]
    fn vtr_with_foreign_extension_bypasses_converter_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("already converted.fst");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../volna-trace/tests/fixtures/values.vtr"),
            &path,
        )
        .unwrap();
        let session = open(&path, &AtomicBool::new(false), || {
            panic!("VTR must open directly")
        })
        .unwrap();
        assert_eq!(session.info().name, "already converted.fst");
        assert!(session.info().signal_count > 0);
    }

    #[test]
    fn discovery_prefers_installation_and_reports_missing_converter() {
        let directory = tempfile::tempdir().unwrap();
        let installed = directory.path().join("installed");
        let on_path = directory.path().join("on path");
        std::fs::create_dir(&installed).unwrap();
        std::fs::create_dir(&on_path).unwrap();
        let executable = installed.join("volna");
        let tool = format!("vtr{}", std::env::consts::EXE_SUFFIX);
        let search_path = std::env::join_paths([&on_path]).unwrap();
        assert!(
            resolve_converter(&executable, Some(&search_path))
                .unwrap_err()
                .to_string()
                .contains("VOLNA_VTR")
        );
        std::fs::write(on_path.join(&tool), "").unwrap();
        assert_eq!(
            resolve_converter(&executable, Some(&search_path)).unwrap(),
            on_path.join(&tool)
        );
        std::fs::write(installed.join(&tool), "").unwrap();
        assert_eq!(
            resolve_converter(&executable, Some(&search_path)).unwrap(),
            installed.join(&tool)
        );
        let error = open(&fixture("values.fst"), &AtomicBool::new(false), || {
            Ok(directory.path().join("missing tool"))
        })
        .err()
        .unwrap();
        assert!(format!("{error:#}").contains("Cannot run converter"));
    }

    #[test]
    fn superseded_and_closed_opens_cancel_only_their_imports() {
        let mut app = volna_core::App::new();
        let mut imports = Imports::default();
        app.open_path(fixture("values.fst"));
        let first = imports.prepare(app.take_requests().remove(0));
        app.open_path(fixture("pipeline.kanata"));
        imports.retain_current(&app.doc);
        let second = imports.prepare(app.take_requests().remove(0));
        assert!(imports.converting(TraceId::A));
        let stale = first();
        imports.finish(&stale);
        app.deliver(stale);
        assert!(imports.converting(TraceId::A));
        assert!(!app.doc.is_loaded());
        app.close_trace();
        imports.retain_current(&app.doc);
        assert!(!imports.converting(TraceId::A));
        let LoadResult::Opened { result, .. } = second() else {
            panic!("open result")
        };
        assert!(result.err().unwrap().to_string().contains("cancelled"));
        assert!(app.doc.traces().is_empty());
    }

    #[test]
    #[ignore = "requires a built vtr CLI; set VOLNA_VTR and run with --ignored"]
    fn real_converter_imports_fst_and_compressed_kanata_and_restores_original_paths() {
        use std::io::Write;
        use volna_core::workspace::{
            Workspace,
            persistence::{Persistence, Target},
        };
        let directory = tempfile::tempdir().unwrap();
        let fst = directory.path().join("recording ; $() ' test.FST");
        std::fs::copy(fixture("values.fst"), &fst).unwrap();
        let kanata = directory.path().join("pipeline.LOG.GZ");
        let mut gzip = flate2::write::GzEncoder::new(
            std::fs::File::create(&kanata).unwrap(),
            flate2::Compression::default(),
        );
        gzip.write_all(&std::fs::read(fixture("pipeline.kanata")).unwrap())
            .unwrap();
        gzip.finish().unwrap();
        let mut app = volna_core::App::new();
        app.configure_persistence(Persistence::Auto);
        let mut imports = Imports::default();
        let fst_uri = crate::native_workspace::file_uri(&fst).unwrap();
        app.open_resource(OpenSpec::Path(fst.clone()), fst_uri.clone());
        drain(&mut app, &mut imports);
        assert_eq!(app.workspace.trace_uri.as_deref(), Some(fst_uri.as_str()));
        assert_eq!(
            app.doc.session(TraceId::A).unwrap().info().name,
            fst.file_name().unwrap().to_string_lossy()
        );
        let other = directory.path().join("other.fst");
        std::fs::copy(fixture("values-wrapped.fst"), &other).unwrap();
        let other_uri = crate::native_workspace::file_uri(&other).unwrap();
        app.add_resource(OpenSpec::Path(other.clone()), other_uri.clone());
        drain(&mut app, &mut imports);
        let saved_fst = Workspace::capture(&app, |slot| Ok(slot.uri.clone()), None).unwrap();
        assert_eq!(saved_fst.traces.len(), 2);
        assert_eq!(saved_fst.traces[1].path, other_uri);
        let mut reopened = volna_core::App::new();
        reopened.configure_persistence(Persistence::Auto);
        reopened.open_resource(OpenSpec::Path(fst), fst_uri);
        drain(&mut reopened, &mut imports);
        reopened
            .open_workspace(
                Target::File {
                    uri: crate::native_workspace::file_uri(
                        &directory.path().join("fst.volna.json"),
                    )
                    .unwrap(),
                },
                &saved_fst.to_bytes().unwrap(),
            )
            .unwrap();
        drain(&mut reopened, &mut imports);
        assert_eq!(reopened.doc.traces().len(), 2);
        assert!(
            reopened
                .doc
                .traces()
                .iter()
                .all(|slot| slot.session().unwrap().format() == Some("VTR"))
        );
        assert_eq!(
            reopened.doc.traces().iter().nth(1).unwrap().source,
            other.to_string_lossy()
        );
        // Kanata uses cycles, so exercise it in its own workspace rather than mixing time units.
        let mut app = volna_core::App::new();
        app.configure_persistence(Persistence::Auto);
        app.open_path(fixture("pipeline.kanata"));
        drain(&mut app, &mut imports);
        assert!(!app.doc.session(TraceId::A).unwrap().tracks().is_empty());
        app.open_resource(
            OpenSpec::Path(kanata.clone()),
            crate::native_workspace::file_uri(&kanata).unwrap(),
        );
        drain(&mut app, &mut imports);
        let session = app.doc.session(TraceId::A).unwrap();
        assert!(!session.tracks().is_empty());
        assert_eq!(session.info().time_unit.as_deref(), Some("cycle"));
        let saved = Workspace::capture(&app, |slot| Ok(slot.uri.clone()), None).unwrap();
        assert_eq!(
            saved.traces[0].path,
            crate::native_workspace::file_uri(&kanata).unwrap()
        );
        assert_eq!(saved.traces[0].name, "pipeline.LOG.GZ");
        let bytes = saved.to_bytes().unwrap();
        app.close_trace();
        app.open_resource(
            OpenSpec::Path(kanata.clone()),
            crate::native_workspace::file_uri(&kanata).unwrap(),
        );
        drain(&mut app, &mut imports);
        let target = Target::File {
            uri: crate::native_workspace::file_uri(&directory.path().join("saved.volna.json"))
                .unwrap(),
        };
        app.open_workspace(target, &bytes).unwrap();
        drain(&mut app, &mut imports);
        assert!(app.doc.is_loaded());
        assert_eq!(
            app.doc.session(TraceId::A).unwrap().info().name,
            "pipeline.LOG.GZ"
        );
        let corrupt = directory.path().join("bad.fst");
        std::fs::write(&corrupt, b"not a recording").unwrap();
        let error = open(&corrupt, &AtomicBool::new(false), converter_path)
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("Cannot convert"));
    }

    fn drain(app: &mut volna_core::App, imports: &mut Imports) {
        loop {
            let requests = app.take_requests();
            if requests.is_empty() {
                break;
            }
            for request in requests {
                let result = imports.prepare(request)();
                if let LoadResult::Opened {
                    result: Err(error), ..
                } = &result
                {
                    panic!("open failed: {error:#}");
                }
                imports.finish(&result);
                app.deliver(result);
            }
        }
    }

    #[cfg(unix)]
    mod process {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn quote(path: &Path) -> String {
            format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
        }

        fn script(directory: &Path, body: &str) -> PathBuf {
            let path = directory.join("mock vtr");
            std::fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        }

        #[test]
        fn arguments_are_literal_and_temporary_files_follow_shared_readers() {
            let directory = tempfile::tempdir().unwrap();
            let input = directory.path().join("import ';$(touch unwanted).fst");
            std::fs::copy(fixture("values.fst"), &input).unwrap();
            let output_log = directory.path().join("output");
            let expected = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../volna-trace/tests/fixtures/values.vtr");
            let tool = script(
                directory.path(),
                &format!(
                    "[ \"$1\" = convert ]\n[ \"$2\" = -- ]\n[ \"$3\" = {} ]\nprintf '%s' \"$4\" > {}\ncp {} \"$4\"",
                    quote(&input),
                    quote(&output_log),
                    quote(&expected)
                ),
            );
            let a = open(&input, &AtomicBool::new(false), || Ok(tool.clone())).unwrap();
            let a_path = PathBuf::from(std::fs::read_to_string(&output_log).unwrap());
            let b = open(&input, &AtomicBool::new(false), || Ok(tool)).unwrap();
            let b_path = PathBuf::from(std::fs::read_to_string(&output_log).unwrap());
            assert_ne!(a_path, b_path);
            let hierarchy = a.hierarchy().clone();
            a.build_activity(
                &vtr::activity::BuildOptions {
                    control: Some(Arc::new(vtr::activity::BuildControl::default())),
                    ..Default::default()
                },
                &volna_trace::remote::memory::MemoryBudget::new(u64::MAX),
                Some(&directory.path().join("persistent cache")),
            )
            .unwrap();
            assert!(a_path.with_extension("vtr.index").exists());
            assert!(!directory.path().join("persistent cache").exists());
            drop(a);
            assert!(
                a_path.exists(),
                "shared hierarchy still has a mapped reader"
            );
            drop(hierarchy);
            assert!(!a_path.parent().unwrap().exists());
            assert!(b_path.exists());
            drop(b);
            assert!(!b_path.parent().unwrap().exists());
        }

        #[test]
        fn failed_converter_cleans_partial_output_and_returns_diagnostics() {
            let directory = tempfile::tempdir().unwrap();
            let output_log = directory.path().join("output");
            let tool = script(
                directory.path(),
                &format!(
                    "printf '%s' \"$4\" > {}\nprintf partial > \"$4\"\nprintf 'broken input: invalid section' >&2\nexit 2",
                    quote(&output_log)
                ),
            );
            let error = open(&fixture("values.fst"), &AtomicBool::new(false), || Ok(tool))
                .err()
                .unwrap();
            assert!(error.to_string().contains("broken input: invalid section"));
            let output = PathBuf::from(std::fs::read_to_string(output_log).unwrap());
            assert!(!output.parent().unwrap().exists());
        }

        #[test]
        fn successful_exit_without_valid_vtr_is_rejected_and_cleaned() {
            let directory = tempfile::tempdir().unwrap();
            let output_log = directory.path().join("output");
            let tool = script(
                directory.path(),
                &format!(
                    "printf '%s' \"$4\" > {}\nprintf garbage > \"$4\"",
                    quote(&output_log)
                ),
            );
            let error = open(&fixture("values.fst"), &AtomicBool::new(false), || Ok(tool))
                .err()
                .unwrap();
            assert!(format!("{error:#}").contains("did not produce a readable VTR"));
            let output = PathBuf::from(std::fs::read_to_string(output_log).unwrap());
            assert!(!output.parent().unwrap().exists());
        }

        #[test]
        fn cancellation_kills_reaps_and_cleans_running_converter() {
            let directory = tempfile::tempdir().unwrap();
            let output_log = directory.path().join("output");
            let tool = script(
                directory.path(),
                &format!(
                    "printf '%s' \"$4\" > {}\nprintf partial > \"$4\"\nexec sleep 30",
                    quote(&output_log)
                ),
            );
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let worker =
                std::thread::spawn(move || open(&fixture("values.fst"), &flag, || Ok(tool)));
            let start = std::time::Instant::now();
            while !output_log.exists() {
                assert!(start.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(10));
            }
            cancel.store(true, Ordering::Relaxed);
            assert!(
                worker
                    .join()
                    .unwrap()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("cancelled")
            );
            assert!(start.elapsed() < Duration::from_secs(10));
            let output = PathBuf::from(std::fs::read_to_string(output_log).unwrap());
            assert!(!output.parent().unwrap().exists());
        }
    }
}
