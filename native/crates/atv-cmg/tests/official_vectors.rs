use atv_cmg::{CmgSession, Error, Protection};

fn hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap())
        .collect()
}

fn compare_official_vectors(signal: u8, fixture: &str) {
    let mut count = 0;
    for (index, line) in fixture.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let (input, expected) = line.split_once('\t').unwrap();
        let mut session = CmgSession::new("https://www.yangshipin.cn");
        assert_eq!(
            session.transform_nal(&[0x67, 0x64, signal]).unwrap(),
            [0x67, 0x64, 0]
        );
        assert_eq!(
            session.transform_nal(&hex(input)).unwrap(),
            hex(expected),
            "oracle vector line {index}"
        );
        count += 1;
    }
    assert_eq!(count, 77);
}

#[test]
fn synthetic_inputs_match_official_mode1_byte_for_byte() {
    compare_official_vectors(1, include_str!("fixtures/live-mode1.tsv"));
}

#[test]
fn synthetic_inputs_match_official_mode2_byte_for_byte() {
    compare_official_vectors(2, include_str!("fixtures/live-mode2.tsv"));
}

#[test]
fn unsupported_vod_does_not_reach_the_standard_decoder() {
    let mut session = CmgSession::new("https://service.invalid/path?token=never-log-me");
    assert!(!format!("{session:?}").contains("never-log-me"));
    assert_eq!(
        session.transform_nal(&[0x19, 1]),
        Err(Error::TransformRequired {
            protection: Protection::Vod,
            nal_type: 25,
        })
    );
    assert!(session.transform_nal(&[0x65, 0xaa]).is_err());
}

#[test]
fn annex_b_output_does_not_expose_an_interior_start_code() {
    let fixture = include_str!("fixtures/live-mode1.tsv");
    let mut checked = 0;
    for line in fixture
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        let (input, _) = line.split_once('\t').unwrap();
        let input = hex(input);
        if input.len() <= 128 {
            continue;
        }
        let mut session = CmgSession::new("");
        session.transform_nal(&[0x67, 0x64, 1]).unwrap();
        let output = session.transform_annex_b_nal(&input).unwrap();
        assert!(
            !output
                .windows(3)
                .any(|window| window == [0, 0, 1] || window == [0, 0, 0])
        );
        checked += 1;
    }
    assert!(checked >= 30);
}

#[test]
fn restored_encoder_escape_is_not_escaped_twice() {
    let fixture = include_str!("fixtures/live-mode1.tsv");
    // Vector 69 decrypts its first sparse block to 00 00 03 01 04 05 06 07.
    let line = fixture
        .lines()
        .filter(|line| !line.starts_with('#'))
        .nth(69)
        .unwrap();
    let (input, expected) = line.split_once('\t').unwrap();
    let mut session = CmgSession::new("");
    session.transform_nal(&[0x67, 0x64, 1]).unwrap();
    let output = session.transform_annex_b_nal(&hex(input)).unwrap();
    assert_eq!(&output[32..40], &[0, 0, 3, 1, 4, 5, 6, 7]);
    assert_eq!(&hex(expected)[32..40], &[0, 0, 3, 1, 4, 5, 6, 7]);
}
