use super::*;
use crate::{
    adapter::ServerVersion,
    buf::Writer,
    packets::{Empty, configuration as cfg, handshake, login, play},
};
use std::{
    net::TcpListener,
    sync::{Barrier, atomic::AtomicBool},
    thread,
};

fn play47(mut connection: Connection) -> Connection {
    connection.adapt(ServerVersion::V1_8_9);
    let session = connection.reader.adapter.as_ref().unwrap();
    session
        .serverbound(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: 777,
                host: "127.0.0.1".into(),
                port: 25565,
                intent: handshake::LOGIN,
            })
            .encode(),
        )
        .unwrap();
    let mut profile = Writer::new();
    profile.string("00000000-0000-0000-0000-000000000001");
    profile.string("Poll47");
    session.clientbound(2, profile.data).unwrap();
    session
        .serverbound(&login::Serverbound::LoginAcknowledged(Empty).encode())
        .unwrap();
    let mut join = Writer::new();
    join.i32(42);
    join.u8(1);
    join.i8(0);
    join.u8(2);
    join.u8(20);
    join.string("default");
    join.bool(false);
    session.clientbound(1, join.data).unwrap();
    while session.next_incoming().is_some() {}
    session
        .serverbound(&cfg::Serverbound::FinishConfiguration(Empty).encode())
        .unwrap();
    session.take_outgoing();
    connection
}
fn swap() -> Vec<u8> {
    play::Serverbound::PlayerAction(play::PlayerAction {
        action: 7,
        pos: [0; 3],
        direction: 0,
        sequence: 0,
    })
    .encode()
}
fn pipe47() -> (Connection, Pipe) {
    let (client, server) = Pipe::pair();
    (play47(Connection::local(client)), server)
}
fn notice_on_quiet_connection(connection: Connection, timeout: Option<Duration>) -> ConnReader {
    let (mut reader, mut writer) = connection.split();
    reader.set_read_timeout(timeout).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = barrier.clone();
    let worker = thread::spawn(move || {
        worker_barrier.wait();
        loop {
            let packet = reader.recv().unwrap();
            if packet.0 == crate::ids::play::clientbound::SYSTEM_CHAT {
                return reader;
            }
        }
    });
    barrier.wait();
    thread::sleep(Duration::from_millis(40));
    let start = Instant::now();
    writer.send(&swap()).unwrap();
    let reader = worker.join().unwrap();
    eprintln!("quiet local notice delivered in {:?}", start.elapsed());
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "local notice delayed {:?}",
        start.elapsed()
    );
    assert_eq!(reader.stream.read_timeout().unwrap(), timeout);
    assert_eq!(reader.read_timing.lock().unwrap().requested, timeout);
    reader
}

#[test]
fn local_rejection_notice_wakes_a_quiet_pipe_and_restores_finite_timeout() {
    let (connection, _server) = pipe47();
    notice_on_quiet_connection(connection, Some(Duration::from_secs(2)));
}
#[test]
fn local_rejection_notice_wakes_quiet_loopback_tcp_and_restores_no_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let connection = Connection::connect(
        "127.0.0.1",
        listener.local_addr().unwrap().port(),
        Duration::from_secs(2),
    )
    .unwrap();
    let (_server, _) = listener.accept().unwrap();
    notice_on_quiet_connection(play47(connection), None);
}
#[test]
fn repeated_local_input_does_not_extend_the_requested_network_timeout() {
    let (connection, _server) = pipe47();
    let (mut reader, mut writer) = connection.split();
    // Start with real queued actions so thread scheduling cannot turn this into
    // a silent-peer-only test before the repeated sender gets CPU time.
    for _ in 0..32 {
        writer.send(&swap()).unwrap();
    }
    let requested = Duration::from_millis(180);
    reader.set_read_timeout(Some(requested)).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = stop.clone();
    let sender = thread::spawn(move || {
        while !writer_stop.load(Ordering::Acquire) {
            writer.send(&swap()).unwrap();
            thread::sleep(Duration::from_millis(5));
        }
    });
    let start = Instant::now();
    let mut packets = 0;
    loop {
        match reader.recv() {
            Ok(_) => packets += 1,
            Err(error) => {
                assert!(error.is_timed_out(), "{error}");
                break;
            }
        }
    }
    stop.store(true, Ordering::Release);
    sender.join().unwrap();
    eprintln!(
        "network timeout after {:?} with {packets} queued local packets",
        start.elapsed()
    );
    assert!(packets >= 32);
    assert!(start.elapsed() >= Duration::from_millis(120));
    assert!(
        start.elapsed() < Duration::from_millis(700),
        "local input defeated timeout {:?}",
        start.elapsed()
    );
    assert_eq!(reader.stream.read_timeout().unwrap(), Some(requested));
}
#[test]
fn partial_frame_survives_poll_slices_and_real_progress_resets_budget() {
    let (mut connection, mut server) = pipe47();
    connection
        .reader
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let sender = thread::spawn(move || {
        for byte in [2, 0, 123] {
            thread::sleep(Duration::from_millis(100));
            server.write_all(&[byte]).unwrap();
        }
        server
    });
    let start = Instant::now();
    let (id, payload) = connection.recv().unwrap();
    let _server = sender.join().unwrap();
    assert!(start.elapsed() >= Duration::from_millis(250));
    assert_eq!(id, crate::ids::play::clientbound::KEEP_ALIVE);
    assert_eq!(Reader::new(&payload).i64().unwrap(), 123);
    assert_eq!(
        connection.reader.stream.read_timeout().unwrap(),
        Some(Duration::from_millis(200))
    );
}

#[test]
fn explicit_retry_after_a_reported_timeout_preserves_the_partial_frame() {
    let (mut connection, mut server) = pipe47();
    let requested = Duration::from_millis(70);
    connection.reader.set_read_timeout(Some(requested)).unwrap();
    server.write_all(&[2]).unwrap();
    assert!(connection.recv().unwrap_err().is_timed_out());
    assert_eq!(
        connection.reader.stream.read_timeout().unwrap(),
        Some(requested)
    );
    server.write_all(&[0, 77]).unwrap();
    let (id, payload) = connection.recv().unwrap();
    assert_eq!(id, crate::ids::play::clientbound::KEEP_ALIVE);
    assert_eq!(Reader::new(&payload).i64().unwrap(), 77);
    assert_eq!(
        connection.reader.stream.read_timeout().unwrap(),
        Some(requested)
    );
}
#[test]
fn restored_timeout_allows_raw_reads_longer_than_a_poll_slice() {
    let (connection, mut server) = pipe47();
    let mut reader = notice_on_quiet_connection(connection, Some(Duration::from_secs(2)));
    let sender = thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        server.write_all(&[18, 47]).unwrap();
        server
    });
    assert_eq!(reader.read_raw(2).unwrap(), [18, 47]);
    let _server = sender.join().unwrap();
}
#[test]
fn modern_pipe_retains_its_original_timeout_behavior() {
    let (client, _server) = Pipe::pair();
    let mut connection = Connection::local(client);
    let requested = Duration::from_millis(80);
    connection.reader.set_read_timeout(Some(requested)).unwrap();
    let start = Instant::now();
    let error = connection.recv().unwrap_err();
    assert!(error.is_timed_out());
    assert!(start.elapsed() >= Duration::from_millis(60));
    assert_eq!(
        connection.reader.stream.read_timeout().unwrap(),
        Some(requested)
    );
}
#[test]
fn another_legacy_protocol_retains_its_original_timeout_behavior() {
    let (client, _server) = Pipe::pair();
    let mut connection = Connection::local(client);
    connection.adapt(ServerVersion::V1_20_1);
    let requested = Duration::from_millis(80);
    connection.reader.set_read_timeout(Some(requested)).unwrap();
    let start = Instant::now();
    let error = connection.recv().unwrap_err();
    assert!(error.is_timed_out());
    assert!(start.elapsed() >= Duration::from_millis(60));
    assert_eq!(
        connection.reader.stream.read_timeout().unwrap(),
        Some(requested)
    );
}
