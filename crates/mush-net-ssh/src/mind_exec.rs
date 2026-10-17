//! Narrow public-key exec framing. This module has no command shell or paths.
use mush_proto::{InvokeRequest, MAX_INVOKE_BYTES, MindRequest, validate_invoke_json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MindExec {
    Push,
    Pull,
    Check,
}
impl MindExec {
    pub(crate) fn parse(command: &[u8]) -> Option<Self> {
        match command {
            b"mind push" => Some(Self::Push),
            b"mind pull" => Some(Self::Pull),
            b"mind check" => Some(Self::Check),
            _ => None,
        }
    }
    fn allows(self, request: &MindRequest) -> bool {
        matches!(
            (self, request),
            (
                Self::Push,
                MindRequest::Replace { .. } | MindRequest::Save { .. } | MindRequest::Create { .. }
            ) | (Self::Pull, MindRequest::Read { .. })
                | (Self::Check, MindRequest::Check { .. })
        )
    }
}

/// At most one JSON request, assembled over arbitrary SSH data packet boundaries.
pub(crate) struct ExecInput {
    command: MindExec,
    bytes: Vec<u8>,
}
impl ExecInput {
    pub(crate) fn new(command: MindExec) -> Self {
        Self {
            command,
            bytes: Vec::new(),
        }
    }
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Result<(), &'static str> {
        if self.bytes.len().saturating_add(chunk.len()) > MAX_INVOKE_BYTES {
            self.bytes.clear();
            return Err("Input exceeds 272 KiB.");
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }
    pub(crate) fn finish(self) -> Result<InvokeRequest, &'static str> {
        validate_invoke_json(&self.bytes)?;
        let request: MindRequest = serde_json::from_slice(&self.bytes)
            .map_err(|_| "Expected one typed mind JSON request.")?;
        if !self.command.allows(&request) {
            return Err("Operation does not match the SSH command.");
        }
        let request = InvokeRequest::Mind(request);
        request.validate()?;
        Ok(request)
    }
}

pub(crate) fn owned_character<'a>(
    characters: &'a [crate::hooks::CharSummary],
    requested: Option<&str>,
) -> Option<&'a crate::hooks::CharSummary> {
    match requested {
        Some(name) => characters
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name)),
        None => match characters {
            [one] => Some(one),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_command_vocabulary_has_no_shell_or_path_language() {
        for bad in [
            b"mind pull x".as_slice(),
            b"mind pull;id",
            b"mind  pull",
            b"mind pull\n",
            b"sh",
            b"mind push /tmp/x",
        ] {
            assert!(MindExec::parse(bad).is_none());
        }
        assert_eq!(MindExec::parse(b"mind check"), Some(MindExec::Check));
    }
    #[test]
    fn split_utf8_and_json_chunks_are_preserved_and_second_document_refused() {
        let bytes = r#"{"op":"read","name":"alpha","revision":"3"}"#.as_bytes();
        for split in 0..=bytes.len() {
            let mut input = ExecInput::new(MindExec::Pull);
            input
                .feed(bytes.get(..split).expect("prefix"))
                .expect("first");
            input
                .feed(bytes.get(split..).expect("suffix"))
                .expect("second");
            assert!(matches!(
                input.finish(),
                Ok(InvokeRequest::Mind(MindRequest::Read { .. }))
            ));
        }
        let mut input = ExecInput::new(MindExec::Pull);
        input
            .feed(br#"{"op":"list"} {"op":"list"}"#)
            .expect("bytes");
        assert!(input.finish().is_err());
    }
    #[test]
    fn oversized_chunk_and_cross_operation_fail_before_dispatch() {
        let mut input = ExecInput::new(MindExec::Push);
        assert!(input.feed(&vec![b'x'; MAX_INVOKE_BYTES + 1]).is_err());
        let mut input = ExecInput::new(MindExec::Check);
        input
            .feed(br#"{"op":"read","name":"mine","revision":null}"#)
            .expect("bytes");
        assert!(input.finish().is_err());
    }
    #[test]
    fn selection_never_falls_back_when_requested_character_is_foreign() {
        let owned = vec![crate::hooks::CharSummary {
            key: 7,
            name: "One".into(),
        }];
        assert_eq!(owned_character(&owned, None).map(|c| c.key), Some(7));
        assert_eq!(owned_character(&owned, Some("one")).map(|c| c.key), Some(7));
        assert!(owned_character(&owned, Some("Other")).is_none());
        let two = vec![
            owned.first().expect("owned").clone(),
            crate::hooks::CharSummary {
                key: 8,
                name: "Two".into(),
            },
        ];
        assert!(owned_character(&two, None).is_none());
    }
}
