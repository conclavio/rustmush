//! Doc 43 cut TS.4: the boundary's reader against hostile bytes.
//!
//! The gateway and the server each read the other's frames; a peer that is
//! buggy, half-upgraded or killed mid-write must come back as an error the
//! link tears down on, never a panic and never a hang. Three shapes: a corrupt
//! body behind a valid length prefix, an EOF inside a frame (inside the prefix
//! or inside the body), and arbitrary bytes in arbitrary chunking.

use std::io;

use mush_ipc::codec::LEN_PREFIX;
use mush_ipc::{
    CodecError, FrameDecoder, FrameReader, MAX_FRAME_LEN, ToGateway, ToServer, decode_frame,
    encode_frame,
};
use proptest::prelude::*;

use crate::samples::{every_to_gateway, every_to_server};

/// Every sample of both directions, encoded as one frame each.
fn sample_frames() -> Vec<Vec<u8>> {
    let mut frames: Vec<Vec<u8>> = every_to_server()
        .iter()
        .map(|m| encode_frame(m).expect("encode ToServer"))
        .collect();
    frames.extend(
        every_to_gateway()
            .iter()
            .map(|m| encode_frame(m).expect("encode ToGateway")),
    );
    frames
}

/// Decode `bytes` as either direction through the streaming decoder, fed in
/// `chunk`-byte pieces, until it needs more bytes or faults. Must not panic.
fn drain_both(bytes: &[u8], chunk: usize) {
    let chunk = chunk.max(1);
    let mut server = FrameDecoder::new();
    let mut gateway = FrameDecoder::new();
    let mut server_dead = false;
    let mut gateway_dead = false;
    for piece in bytes.chunks(chunk) {
        server.feed(piece);
        gateway.feed(piece);
        while !server_dead {
            match server.next_message::<ToServer>() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => server_dead = true,
            }
        }
        while !gateway_dead {
            match gateway.next_message::<ToGateway>() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => gateway_dead = true,
            }
        }
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a current-thread runtime")
}

/// Read every frame `bytes` holds as `T`: the count read and how the stream
/// ended.
fn read_all<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> (usize, io::Result<()>) {
    runtime().block_on(async {
        let mut reader = FrameReader::new(bytes);
        let mut read = 0usize;
        loop {
            match reader.read::<T>().await {
                Ok(Some(_)) => read += 1,
                Ok(None) => return (read, Ok(())),
                Err(e) => return (read, Err(e)),
            }
        }
    })
}

/// A complete frame followed by the next one cut at every byte: the first
/// always decodes; the cut one is needs-more-bytes in the codec and an
/// `UnexpectedEof` from the reader (a cut at 0 is a clean EOF).
#[test]
fn eof_mid_frame_is_unexpected_eof_at_every_cut() {
    let lead = encode_frame(&ToGateway::Bye).expect("encode");
    for next in sample_frames() {
        for cut in 0..next.len() {
            assert!(
                matches!(decode_frame::<ToServer>(&next[..cut]), Ok(None)),
                "a {cut}-byte prefix of a frame is needs-more-bytes"
            );
            let mut stream = lead.clone();
            stream.extend_from_slice(&next[..cut]);
            let (read, end) = read_all::<ToGateway>(&stream);
            assert_eq!(read, 1, "the whole leading frame is read");
            if cut == 0 {
                assert!(end.is_ok(), "a cut at a frame boundary is a clean EOF");
            } else {
                let err = end.expect_err("a cut frame ends in an error");
                assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof, "cut {cut}");
            }
        }
    }
}

/// A valid prefix whose body is not the message: the reader faults with
/// `InvalidData` after the frames before it; it neither panics nor waits.
#[test]
fn corrupt_body_after_a_valid_prefix_is_invalid_data() {
    let lead = encode_frame(&ToGateway::Bye).expect("encode");
    let bodies: [&[u8]; 6] = [
        // The one byte MessagePack never assigns.
        &[0xc1],
        // array32 claiming 4 G elements, none present.
        &[0xdd, 0xff, 0xff, 0xff, 0xff],
        // str32 claiming 4 GiB, one byte present.
        &[0xdb, 0xff, 0xff, 0xff, 0xff, b'x'],
        // fixarray(2) with one element.
        &[0x92, 0x01],
        b"not msgpack at all",
        // An empty map: well-formed MessagePack, the wrong shape.
        &[0x80],
    ];
    for body in bodies {
        let mut stream = lead.clone();
        let len = u32::try_from(body.len()).expect("small body");
        stream.extend_from_slice(&len.to_be_bytes());
        stream.extend_from_slice(body);
        // A valid frame after the corrupt one is never reached.
        stream.extend_from_slice(&lead);
        let corrupt = &stream[lead.len()..];
        assert!(
            matches!(
                decode_frame::<ToGateway>(corrupt),
                Err(CodecError::Decode(_))
            ),
            "body {body:02x?} is a decode fault"
        );
        let (read, end) = read_all::<ToGateway>(&stream);
        assert_eq!(read, 1, "the leading frame is read, the corrupt one is not");
        assert_eq!(
            end.expect_err("corrupt body").kind(),
            io::ErrorKind::InvalidData
        );
        let (read, end) = read_all::<ToServer>(corrupt);
        assert_eq!(read, 0);
        assert_eq!(
            end.expect_err("corrupt body").kind(),
            io::ErrorKind::InvalidData
        );
    }
}

/// An oversized declared length faults on the prefix alone: the reader never
/// waits for, or buffers, a body it would refuse.
#[test]
fn oversized_prefix_faults_without_the_body() {
    let len = u32::try_from(MAX_FRAME_LEN + 1).expect("fits u32");
    let (read, end) = read_all::<ToServer>(&len.to_be_bytes());
    assert_eq!(read, 0);
    assert_eq!(
        end.expect_err("oversize").kind(),
        io::ErrorKind::InvalidData
    );
    let (_, end) = read_all::<ToGateway>(&u32::MAX.to_be_bytes());
    assert_eq!(
        end.expect_err("oversize").kind(),
        io::ErrorKind::InvalidData
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Arbitrary bytes in arbitrary chunking never panic either decoder.
    #[test]
    fn arbitrary_bytes_never_panic(
        bytes in proptest::collection::vec(any::<u8>(), 0..2048),
        chunk in 1usize..64,
    ) {
        drain_both(&bytes, chunk);
    }

    /// A real frame's body with bytes overwritten, behind its own valid
    /// prefix, decodes to something or faults: never a panic.
    #[test]
    fn corrupted_sample_bodies_never_panic(
        pick in any::<prop::sample::Index>(),
        flips in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
        chunk in 1usize..64,
    ) {
        let frames = sample_frames();
        let mut frame = pick.get(&frames).clone();
        if frame.len() > LEN_PREFIX {
            let body = frame.len() - LEN_PREFIX;
            for (at, byte) in flips {
                frame[LEN_PREFIX + at.index(body)] = byte;
            }
        }
        drain_both(&frame, chunk);
        let _ = decode_frame::<ToServer>(&frame);
        let _ = decode_frame::<ToGateway>(&frame);
    }
}
