`volna-trace` loads immutable VTR recordings from local files, in-memory images, or remote servers through a common API.
It offers hierarchy metadata, complete signal histories, transaction tracks, and activity indexes, with memory budgeting
and cooperative remote decoding. Hosts manage background execution, server connections, and packet transport.
Convert FST and Kanata to VTR before loading. Initialize the VTR submodule, then run
`cargo test --locked -p volna-trace --all-targets` from the repository root with the pinned Rust toolchain.
