#[path = "../src/server.rs"]
mod server;
use std::sync::Arc;
use volna_trace::remote::{
    ClientStep,
    hierarchy::PAGE_ENTRIES,
    memory::MemoryBudget,
    transport::{Body, Packet},
};
use volna_trace::session::{LoadResult, Session};
use volna_trace::testing::assert_hierarchy_eq;
fn wide(count: usize) -> Arc<dyn Session> {
    let mut h = volna_trace::data::HierarchyBuilder::default();
    let root = h.push_scope("顶层.λ".into(), "module".into(), None);
    for i in 0..count {
        let scope = h.push_scope(format!("cell{i}"), "module".into(), Some(root));
        h.scopes[scope].component = "NAND2_X1".into();
        h.vars.push(volna_trace::data::Variable {
            description: String::new(),
            name: "\\pin.λ".into(),
            scope,
            shape: volna_trace::data::SignalShape::Vector { width: 64 },
            var_type: "wire".into(),
            direction: volna_trace::data::Direction::Input,
            signal: volna_trace::data::SignalRef(0),
            enum_table: Some(42),
        });
    }
    volna_trace::testing::hierarchy_session(h)
}

#[test]
fn production_in_process_transport_matches_local_over_multiple_pages() {
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;
    use volna_trace::remote::client::RemoteClient;
    use volna_trace::remote::transport::{Command, read_packet, write_packet};
    let local = wide(PAGE_ENTRIES + 1);
    let expected = local.clone();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut client_socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server_socket, _) = listener.accept().unwrap();
    for socket in [&client_socket, &server_socket] {
        socket.set_nodelay(true).unwrap();
    }
    for socket in [&client_socket, &server_socket] {
        socket
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(20)))
            .unwrap();
    }
    let worker = std::thread::spawn(move || {
        server::serve(
            server_socket.try_clone().unwrap(),
            server_socket,
            31,
            || Ok(local),
            || Ok(()),
        )
    });
    let budget = MemoryBudget::new(32 * 1024 * 1024);
    let mut client = RemoteClient::new(0, 11, 64 * 1024 * 1024, budget.clone()).unwrap();
    write_packet(&mut client_socket, &client.take_command().unwrap().unwrap()).unwrap();
    let remote = 'opened: loop {
        let mut step = client
            .accept(read_packet(&mut client_socket).unwrap().unwrap())
            .unwrap();
        loop {
            match step {
                ClientStep::Yield => step = client.step().unwrap(),
                ClientStep::Ack(ack) => {
                    write_packet(&mut client_socket, &ack).unwrap();
                    break;
                }
                ClientStep::Complete {
                    ack,
                    result: LoadResult::Opened { result, .. },
                } => {
                    write_packet(&mut client_socket, &ack).unwrap();
                    break 'opened result.unwrap();
                }
                ClientStep::Complete { .. } => panic!("Open"),
            }
        }
    };
    assert_hierarchy_eq(expected.hierarchy(), remote.hierarchy());
    write_packet(
        &mut client_socket,
        &Packet {
            session: 31,
            request: 8,
            sequence: 0,
            body: Body::Command(Command::Close),
        },
    )
    .unwrap();
    drop(client_socket);
    worker.join().unwrap().unwrap();
    drop(remote);
    assert_eq!(budget.used(), 0);
}
