# VTR C API

`vtr-capi` builds `libvtr`, a C ABI over the workspace's `vtr` trace reader and
writer. [`include/vtr.h`](include/vtr.h) is the API reference. The other public
headers provide C++17 logging and transaction helpers and a SystemVerilog DPI
front end. `vtr-guard` provides optional Linux crash and stop handling.

## Build and link

From the workspace root, run `cargo build -p vtr-capi --release --locked`.
The static library is `target/release/libvtr.a`; the shared library is
`target/release/libvtr.so` on Linux (platform names vary). Add `vtr-capi/include`
to the C compiler's include path. On Linux, link the static library with
`-lpthread -ldl -lm`, or link the shared library with `-L target/release -lvtr`
and make it available to the dynamic loader. Enable `--features private-heap`
when the crash guard must remain usable if the application's allocator is
locked by a fault.

## Ownership and errors

The caller owns opaque handles and frees them with their matching close or
free function. `vtr_writer_close` always consumes its writer, even on error.
Status functions return `VTR_OK` or a `VTR_ERR_*` code; constructors return
`NULL` or `VTR_NONE` on failure. Read `vtr_last_error()` on the failing thread
immediately after a failure. Success does not clear the last error. Input
strings are UTF-8 and copied by writer calls; reader results and callback
pointers have the lifetimes described in `vtr.h`.

## Minimal writer

```c
#include "vtr.h"
#include <stdio.h>

int main(void) {
    vtr_writer *w = vtr_writer_create("trace.vtr", NULL);
    if (!w) { fprintf(stderr, "%s\n", vtr_last_error()); return 1; }
    uint32_t top = vtr_writer_add_scope(w, VTR_NONE, "top", VTR_SCOPE_MODULE, NULL);
    uint32_t node, signal;
    int rc = top == VTR_NONE ? VTR_ERR_INVALID :
        vtr_writer_add_var(w, top, "clk", VTR_VAR_WIRE, VTR_DIR_INPUT,
                           VTR_SIGNAL_BITS, 1, 4, &node, &signal);
    if (rc == VTR_OK) rc = vtr_writer_set_time(w, 0);
    if (rc == VTR_OK) rc = vtr_writer_emit_bit(w, signal, VTR_LOGIC_0);
    if (rc == VTR_OK) rc = vtr_writer_set_time(w, 5);
    if (rc == VTR_OK) rc = vtr_writer_emit_bit(w, signal, VTR_LOGIC_1);
    if (rc != VTR_OK) fprintf(stderr, "%s\n", vtr_last_error());
    int close_rc = vtr_writer_close(w);
    if (close_rc != VTR_OK) fprintf(stderr, "%s\n", vtr_last_error());
    return rc == VTR_OK && close_rc == VTR_OK ? 0 : 1;
}
```

Run the C and C++ integration tests with
`cargo test -p vtr-capi --locked` on Linux.
