#!/usr/bin/env python3
"""Regenerate and verify the bound pipeline tracer with the pinned VTR SDK and Verilator."""
import argparse
import difflib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "vtr"
EXPECTED = """pipeline 0100 add a0,a0,a1 [15..75] ok pc=256 unit=alu | F 15-25, D 25-35, Q 35-45, X 45-55, C 55-65, R 65-75 | -> wakeup 0104 lw a2,0(a0)
pipeline 0104 lw a2,0(a0) [25..95] ok pc=260 unit=lsu | F 25-35, D 35-45, Q 45-55, X 55-75, C 75-85, R 85-95 | replay@65
pipeline 0108 add a3,a2,a1 [35..125] ok pc=264 unit=alu | F 35-65, S@stall 45-65, D 65-85, Q 85-95, X 95-105, C 105-115, R 115-125
pipeline 010c sub a4,a3,a0 [45..125] ok pc=268 unit=alu | F 45-75, D 75-85, Q 85-95, X 95-105, C 105-115, R 115-125
bus bus@45 [45..75] ok addr=8192 | A 45-65, D 65-75 | parent=0104 lw a2,0(a0)
pipeline 0110 beq a4,zero,0x200 [75..135] ok pc=272 unit=alu | F 75-85, D 85-95, Q 95-105, X 105-115, C 115-125, R 125-135
pipeline 0114 addi a7,a7,1 [85..115] aborted pc=276 unit=alu | F 85-95, D 95-115
pipeline 0118 addi a7,a7,1 [95..115] aborted pc=280 unit=alu | F 95-105, D 105-115
pipeline 011c addi a7,a7,1 [105..115] aborted pc=284 unit=alu | F 105-115
pipeline 0200 xor a5,a5,a5 [115..175] error pc=512 unit=alu | F 115-125, D 125-135, Q 135-145, X 145-155, C 155-165, R 165-175
pipeline 0204 lw a6,8(a0) [125..145] aborted pc=516 unit=lsu | F 125-135, D 135-145
pipeline 0208 addi a7,a7,1 [135..145] aborted pc=520 unit=alu | F 135-145
bus bus@135 [135..175] aborted addr=12288 | A 135-175 | parent=0200 xor a5,a5,a5
pipeline 0300 fence.i [145..200] open pc=768 unit=alu | F 145-155, D 155-200
"""
WARNING = 'vtr_trace: TOP.tb.core.pipeline: stage on a key that names no item (1 time, first rob 1)'
SOURCES = {"TOP.tb.core.pipeline": "pipeline", "TOP.tb.core.bus": "bus"}


def command(cli, *args):
    return subprocess.check_output([str(cli), *map(str, args)], text=True)


def tick_ns(value):
    ticks = int(value)
    assert ticks % 1000 == 0, f"observation is not on a nanosecond: {ticks}"
    return ticks // 1000


def transactions(cli, trace):
    result, current = [], None
    for line in command(cli, "transactions", trace, "--max", 1000).splitlines():
        match = re.fullmatch(r"record (\d+) (\S+) \[(\d+) \.\. (\d+)\] status=(\w+) kind=(\w+)(?: parent=(\d+))?", line)
        if match:
            current = None
            if match[2] in SOURCES:
                assert match[6] == "Transaction", line
                current = dict(id=int(match[1]), source=SOURCES[match[2]], begin=tick_ns(match[3]), end=tick_ns(match[4]), status=match[5], parent=int(match[7]) if match[7] else None, attrs={}, stages=[], events=[], relations=[])
                result.append(current)
            continue
        if current is None:
            continue
        if match := re.fullmatch(r"    stage (\d+) (\S+) lane=(\S*) \[(\d+) \.\. (\d+|open)\]", line):
            current["stages"].append((int(match[1]), match[2], match[3], tick_ns(match[4]), None if match[5] == "open" else tick_ns(match[5])))
        elif match := re.fullmatch(r"    event (\d+) @(\d+) (\S+)", line):
            current["events"].append((int(match[1]), tick_ns(match[2]), match[3]))
        elif match := re.fullmatch(r"    (\S+) = (.*)", line):
            if match[1] == "payload":
                assert match[2] == "null", line
            else:
                current["attrs"][match[1]] = match[2].strip('"')
    for record in result:
        assert set(record["attrs"]) <= {"pc", "unit", "addr", "vtr.label"}, record
        for line in command(cli, "transactions", trace, "--id", record["id"]).splitlines():
            if match := re.fullmatch(r"    -> (\S+) (\d+)", line):
                record["relations"].append((match[1], int(match[2])))
    return sorted(result, key=lambda record: record["id"])


def canonical(records):
    labels = {r["id"]: r["attrs"].get("vtr.label", f"{r['source']}@{r['begin']}") for r in records}
    lines = []
    for r in records:
        attrs = " ".join(f"{key}={r['attrs'][key]}" for key in ("pc", "unit", "addr") if key in r["attrs"])
        stages = ", ".join(f"{name}{'@' + lane if lane else ''} {begin}-{max(begin, r['end']) if end is None else end}" for _, name, lane, begin, end in r["stages"])
        parts = [f"{r['source']} {labels[r['id']]} [{r['begin']}..{r['end']}] {r['status']} {attrs}", stages]
        extra = [f"{name}@{time}" for _, time, name in r["events"]]
        if r["parent"]:
            extra.append(f"parent={labels[r['parent']]}")
        extra += [f"-> {kind} {labels[target]}" for kind, target in r["relations"]]
        if extra:
            parts.append(" ".join(extra))
        lines.append(" | ".join(parts))
    return "\n".join(lines) + ("\n" if lines else "")


def clock_edges(cli, trace):
    lines = command(cli, "clocks", trace).splitlines()
    assert len(lines) == 2, lines
    source = re.fullmatch(r"clock (\d+) TOP.tb.clk: 1 stretches", lines[0])
    stretch = re.fullmatch(r"    \[(\d+) \.\. (\d+)\] cycles 0 \.\. 19, running at close", lines[1])
    assert source and stretch, lines
    begin, end = map(tick_ns, stretch.groups())
    assert (begin, end) == (5, 195), lines
    return int(source[1]), set(range(begin, end + 1, 10))


def verilate(verilator, env, out, threads, traced):
    obj = out / f"obj_{threads if traced else 'baseline'}"
    sources = [HERE / "core.sv"]
    if traced:
        sources += [HERE / "tracer.sv", HERE / "bind.sv"]
    sources.append(HERE / "main.cpp")
    with (out / f"build_{threads if traced else 'baseline'}.log").open("w") as log:
        subprocess.run([str(verilator), "--cc", "--exe", "--build", "-j", "2", "--trace-vtr", "--timing", "--threads", str(threads), "--top-module", "tb", "--prefix", "Vtb", "--Mdir", str(obj), *map(str, sources), "-o", "sim"], env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    return obj / "sim"


def simulate(binary, folder, execution, bare, env):
    folder.mkdir(parents=True, exist_ok=True)
    with (folder / "console.txt").open("w") as log:
        subprocess.run([str(binary), execution, *(["+nosignals"] if bare else [])], cwd=folder, env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=60)
    return folder / "pipeline.vtr"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verilator", type=Path, default=SDK / "build/verilator/install/bin/verilator")
    parser.add_argument("--out", type=Path, default=ROOT / "atemp/verilator-pipeline")
    parser.add_argument("--update-example", action="store_true")
    args = parser.parse_args()
    out, verilator = args.out.resolve(), args.verilator.resolve()
    out.mkdir(parents=True, exist_ok=True)
    installed = Path(command(verilator, "--getenv", "VERILATOR_ROOT").strip())
    for name in ("vtr_trace.sv", "vtr_trace_dpi.h", "vtr_track.h", "vtr_common.h", "vtr_writer.h"):
        assert (installed / "include/vtr" / name).read_bytes() == (SDK / "vtr-cpp" / name).read_bytes(), f"Rebuild the pinned Verilator installation to update {name}"
    env = dict(os.environ, VTR_INCLUDE=str(SDK / "vtr-cpp"), VTR_LIBDIR=str(SDK / "build/cpp"))
    env.setdefault("CCACHE_DIR", str(out / "ccache"))
    cli = SDK / "target/release/vtr"
    baseline = simulate(verilate(verilator, env, out, 1, False), out / "baseline", "threaded", False, env)
    assert not transactions(cli, baseline)
    command(cli, "to-vcd", baseline, out / "baseline.vcd")
    waves = (out / "baseline.vcd").read_bytes()
    generated, manifest = None, []
    for threads in (1, 2):
        binary = verilate(verilator, env, out, threads, True)
        for execution in ("inline", "threaded"):
            for bare in (False, True):
                name = f"{threads}-{execution}-{'bare' if bare else 'waves'}"
                trace = simulate(binary, out / name, execution, bare, env)
                info = command(cli, "info", trace)
                assert "version:     3.3" in info and "timescale:   1e-12 s" in info, info
                assert "0 unresolved references" in info, info
                records = transactions(cli, trace)
                got = canonical(records)
                assert got == EXPECTED, "".join(difflib.unified_diff(EXPECTED.splitlines(True), got.splitlines(True), "expected", name))
                assert len(records) == 14 and sum(r["source"] == "pipeline" for r in records) == 12
                source, edges = clock_edges(cli, trace)
                for r in records:
                    times = [r["begin"]] + [b for _, _, _, b, _ in r["stages"]]
                    if r["status"] != "open":
                        times += [r["end"]] + [r["end"] if e is None else e for _, _, _, _, e in r["stages"]]
                    assert set(times) <= edges, r
                hierarchy = command(cli, "hierarchy", trace)
                for generator, kind in (("pipeline", "PIPELINE"), ("bus", "BUS")):
                    assert re.search(rf"^      {generator} \[generator {kind} type#0\]\n        @vtr\.clock = {source}$", hierarchy, re.M), hierarchy
                logs = command(cli, "logs", trace)
                assert [line.split("simulation_log: ", 1)[1] for line in logs.splitlines() if " warn " in line] == [WARNING], logs
                assert f"%Warning-VTRTRACE: {WARNING}" in (trace.parent / "console.txt").read_text()
                if bare:
                    assert "0 variables (0 signals)" in info, info
                else:
                    vcd = trace.parent / "waves.vcd"
                    command(cli, "to-vcd", trace, vcd)
                    assert vcd.read_bytes() == waves, "bound tracer changed the design waveforms"
                    if threads == 1 and execution == "threaded":
                        generated = trace
                manifest.append(dict(simulation_threads=threads, execution=execution, signal_free=bare, owners=len(records), trace=str(trace)))
                print(f"{name}: all owners/stages/attributes/statuses/events/parents/relations, clocks, logs and waveform behavior PASS", flush=True)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if args.update_example:
        assert generated is not None
        destination = ROOT / "volna/examples/pipeline_demo.vtr"
        shutil.copyfile(generated, destination)
        print(f"Updated {destination}")


if __name__ == "__main__":
    main()
