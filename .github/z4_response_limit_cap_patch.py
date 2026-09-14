from pathlib import Path

path = Path("src/privileged_request.rs")
text = path.read_text()


def once(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected one match, got {count}: {old[:100]!r}")
    text = text.replace(old, new, 1)


once(
    "pub const MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES: usize = 256;\n",
    "pub const MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES: usize = 256;\n"
    "pub const MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES: usize =\n"
    "    DEFAULT_MAX_RESPONSE_BODY_BYTES;\n",
)
once(
    "    InvalidNetworkResponseBodyLimit,\n",
    "    NetworkResponseBodyLimitOutOfRange {\n        bytes: usize,\n        max: usize,\n    },\n",
)
once(
    "            Self::InvalidNetworkResponseBodyLimit => formatter\n"
    '                .write_str("Network privileged request response-body limit must be non-zero"),\n',
    "            Self::NetworkResponseBodyLimitOutOfRange { bytes, max } => write!(\n"
    "                formatter,\n"
    '                "Network privileged request response-body limit is {bytes} bytes; allowed range is 1..={max} bytes"\n'
    "            ),\n",
)
once(
    "    /// redirect and destination remain canonical pinned-Rarog Fetch values. The response-body limit\n"
    "    /// is retained exactly under Rarog's non-zero `FetchLimits` contract; Zorya does not impose a\n"
    "    /// second cap because retaining this fixed-size scalar allocates no response storage.\n",
    "    /// redirect and destination remain canonical pinned-Rarog Fetch values. The response-body limit\n"
    "    /// is retained exactly under Rarog's non-zero `FetchLimits` contract and Zorya caps the value at\n"
    "    /// the pinned Rarog default so a future execution path cannot inherit an effectively unbounded\n"
    "    /// response budget from correlation state.\n",
)
once(
    "        if max_response_body_bytes == 0 {\n"
    "            return Err(EnginePrivilegedRequestError::InvalidNetworkResponseBodyLimit);\n"
    "        }\n",
    "        if max_response_body_bytes == 0\n"
    "            || max_response_body_bytes > MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES\n"
    "        {\n"
    "            return Err(EnginePrivilegedRequestError::NetworkResponseBodyLimitOutOfRange {\n"
    "                bytes: max_response_body_bytes,\n"
    "                max: MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES,\n"
    "            });\n"
    "        }\n",
)
path.write_text(text)

lib = Path("src/lib.rs")
lib_text = lib.read_text()
once_lib_old = (
    "    MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES, MAX_PRIVILEGED_NETWORK_HEADER_BYTES,\n"
    "    MAX_PRIVILEGED_NETWORK_HEADERS, MAX_PRIVILEGED_NETWORK_TARGET_BYTES,\n"
)
once_lib_new = (
    "    MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES, MAX_PRIVILEGED_NETWORK_HEADER_BYTES,\n"
    "    MAX_PRIVILEGED_NETWORK_HEADERS, MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES,\n"
    "    MAX_PRIVILEGED_NETWORK_TARGET_BYTES,\n"
)
if lib_text.count(once_lib_old) != 1:
    raise SystemExit("lib export marker mismatch")
lib.write_text(lib_text.replace(once_lib_old, once_lib_new, 1))

tests = Path("src/privileged_request_network_response_limit_tests.rs")
t = tests.read_text()
t = t.replace(
    "    assert_eq!(\n        defaulted.max_response_body_bytes(),\n        DEFAULT_MAX_RESPONSE_BODY_BYTES\n    );\n",
    "    assert_eq!(\n        defaulted.max_response_body_bytes(),\n        DEFAULT_MAX_RESPONSE_BODY_BYTES\n    );\n"
    "    assert_eq!(\n        defaulted.max_response_body_bytes(),\n        MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES\n    );\n",
    1,
)
t = t.replace(
    "        Err(EnginePrivilegedRequestError::InvalidNetworkResponseBodyLimit)\n",
    "        Err(EnginePrivilegedRequestError::NetworkResponseBodyLimitOutOfRange {\n"
    "            bytes: 0,\n"
    "            max: MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES,\n"
    "        })\n",
    1,
)
needle = "    assert_eq!(tracker.pending_requests(), 0);\n    assert_eq!(tracker.pending_network_body_bytes(), 0);\n\n    let request = tracker\n"
replacement = "    assert_eq!(tracker.pending_requests(), 0);\n    assert_eq!(tracker.pending_network_body_bytes(), 0);\n\n    assert_eq!(\n        tracker.register_network_with_request_parts_and_response_limit(\n            &current,\n            FetchMethod::post(),\n            \"http://127.0.0.1:1/over-limit\",\n            HeaderList::default(),\n            Some(vec![1, 2, 3, 4]),\n            RequestMode::Cors,\n            CredentialsMode::SameOrigin,\n            RedirectMode::Follow,\n            RequestDestination::Empty,\n            MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES + 1,\n        ),\n        Err(EnginePrivilegedRequestError::NetworkResponseBodyLimitOutOfRange {\n            bytes: MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES + 1,\n            max: MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES,\n        })\n    );\n    assert_eq!(tracker.pending_requests(), 0);\n    assert_eq!(tracker.pending_network_body_bytes(), 0);\n\n    let request = tracker\n"
if t.count(needle) != 1:
    raise SystemExit(f"zero test insertion marker mismatch: {t.count(needle)}")
t = t.replace(needle, replacement, 1)

t = t.replace(
    "            12_345,\n        )\n        .expect(\"register explicit response limit\");\n    assert_eq!(request.max_response_body_bytes(), 12_345);\n",
    "            12_345,\n        )\n        .expect(\"register explicit response limit\");\n    assert_eq!(request.max_response_body_bytes(), 12_345);\n",
    1,
)
# The explicit non-default plus the default/max assertion cover interior and maximum values; the
# zero/over-limit test covers both rejected boundaries, while the valid-after-zero request covers 1.
tests.write_text(t)
