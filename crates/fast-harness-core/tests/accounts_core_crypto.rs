use fast_harness_core::accounts::crypto::*;

#[test]
fn sha256_fips_vectors() {
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(&sha256(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn sha256_million_a() {
    let data = vec![b'a'; 1_000_000];
    assert_eq!(
        hex(&sha256(&data)),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn base64_rfc4648_vectors() {
    let cases = [
        ("", "", ""),
        ("f", "Zg==", "Zg"),
        ("fo", "Zm8=", "Zm8"),
        ("foo", "Zm9v", "Zm9v"),
        ("foob", "Zm9vYg==", "Zm9vYg"),
        ("fooba", "Zm9vYmE=", "Zm9vYmE"),
        ("foobar", "Zm9vYmFy", "Zm9vYmFy"),
    ];
    for (plain, std, url) in cases {
        assert_eq!(base64_encode(plain.as_bytes()), std);
        assert_eq!(base64url_encode(plain.as_bytes()), url);
        assert_eq!(base64_decode(std).unwrap(), plain.as_bytes());
        assert_eq!(base64url_decode(url).unwrap(), plain.as_bytes());
    }
}

#[test]
fn base64_alphabets_differ() {
    assert_eq!(base64_encode(&[0xfb, 0xff]), "+/8=");
    assert_eq!(base64url_encode(&[0xfb, 0xff]), "-_8");
    assert_eq!(base64url_decode("-_8").unwrap(), [0xfb, 0xff]);
}

#[test]
fn base64_round_trip_every_length() {
    let data: Vec<u8> = (0..=255u8).collect();
    for len in 0..data.len() {
        let slice = &data[..len];
        assert_eq!(base64_decode(&base64_encode(slice)).unwrap(), slice);
        assert_eq!(base64url_decode(&base64url_encode(slice)).unwrap(), slice);
    }
}

#[test]
fn base64_decode_errors() {
    assert_eq!(base64_decode("Zm9"), Err(Base64Error::InvalidLength));
    assert_eq!(base64_decode("Zg"), Err(Base64Error::InvalidLength));
    assert_eq!(base64_decode("Zh=="), Err(Base64Error::NonCanonical));
    assert_eq!(base64_decode("Z!=="), Err(Base64Error::InvalidChar(1)));
    assert_eq!(base64_decode("A==="), Err(Base64Error::InvalidChar(1)));
    assert_eq!(base64_decode("Zg==Zg=="), Err(Base64Error::InvalidChar(2)));
    assert_eq!(base64_decode("-_8="), Err(Base64Error::InvalidChar(0)));
    assert_eq!(base64url_decode("Zg=="), Err(Base64Error::InvalidChar(2)));
    assert_eq!(base64url_decode("Z"), Err(Base64Error::InvalidLength));
    assert_eq!(base64url_decode("+/8"), Err(Base64Error::InvalidChar(0)));
    assert_eq!(base64url_decode("Zh"), Err(Base64Error::NonCanonical));
    assert_eq!(
        base64url_decode("Zm9vYmFy\n"),
        Err(Base64Error::InvalidLength)
    );
}

#[test]
fn hex_is_lowercase() {
    assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
}

#[test]
fn pkce_challenge_matches_rfc7636_example() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert_eq!(
        base64url_encode(&sha256(verifier.as_bytes())),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[cfg(windows)]
#[test]
fn pkce_pair_is_fresh_and_consistent() {
    let (verifier, challenge) = pkce_pair().unwrap();
    assert_eq!(verifier.len(), 43);
    assert_eq!(base64url_decode(&verifier).unwrap().len(), 32);
    assert_eq!(challenge, base64url_encode(&sha256(verifier.as_bytes())));
    let (other, _) = pkce_pair().unwrap();
    assert_ne!(verifier, other);
}

#[cfg(windows)]
#[test]
fn random_token_lengths() {
    assert_eq!(random_token(16).unwrap().len(), 22);
    assert_eq!(random_token(0).unwrap(), "");
    assert_ne!(random_token(32).unwrap(), random_token(32).unwrap());
}
