use super::*;
struct Cx;
impl Context for Cx {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag> {
        crate::adapter::read_legacy_component(r)
    }
    fn dimension(&self) -> &str {
        "minecraft:overworld"
    }
    fn block_state(&self, raw: i32, _: [i32; 3]) -> Result<i32> {
        Ok(raw)
    }
    fn block(&self, raw: i32) -> Result<i32> {
        Ok(raw)
    }
    fn block_entity_kind(&self, name: &str) -> Result<i32> {
        assert_eq!(name, "minecraft:sign");
        Ok(7)
    }
    fn interact_at(&self, _: i32) -> Result<bool> {
        Ok(false)
    }
}
fn body(lines: [&str; 4]) -> Vec<u8> {
    let mut w = Writer::new();
    w.i64(position_down([1, 4, 3]).unwrap());
    for s in lines {
        w.string(s);
    }
    w.data
}
fn decode(body: &[u8]) -> Result<Vec<play::Clientbound>> {
    State::new(1)
        .clientbound(0x33, body, &Cx)
        .map(|p| p.unwrap())
}
fn messages(p: &play::Clientbound) -> &Vec<Tag> {
    let play::Clientbound::BlockEntityData(p) = p else {
        panic!("not a block entity")
    };
    let Tag::List(v) = p.tag.get("front_text").unwrap().get("messages").unwrap() else {
        panic!("no messages")
    };
    v
}
#[test]
fn official_null_blank_lines_translate_and_decode_as_native_sign() {
    let p = decode(&body([r#""H47SignBlank""#, "null", "null", "null"]))
        .unwrap()
        .remove(0);
    assert_eq!(
        messages(&p),
        &vec![
            d::string("H47SignBlank"),
            d::string(""),
            d::string(""),
            d::string("")
        ]
    );
    let bytes = p.encode();
    let mut r = Reader::new(&bytes);
    let id = r.varint().unwrap();
    let p = play::Clientbound::decode(id, r.rest()).unwrap();
    assert_eq!(messages(&p)[1], d::string(""));
    let play::Clientbound::BlockEntityData(p) = p else {
        unreachable!()
    };
    assert_eq!(p.pos, [1, 4, 3]);
}
#[test]
fn literals_and_styling_are_not_mistaken_for_null() {
    let p = decode(&body([
        " \t null\r\n",
        r#""null""#,
        r#"{"text":"null","bold":true}"#,
        r#""""#,
    ]))
    .unwrap()
    .remove(0);
    let m = messages(&p);
    assert_eq!(m[0], d::string(""));
    assert_eq!(m[1], d::string("null"));
    assert_eq!(m[2].get("text"), Some(&d::string("null")));
    assert_eq!(m[2].get("bold"), Some(&Tag::Byte(1)));
    assert_eq!(m[3], d::string(""));
}
#[test]
fn truncation_trailing_invalid_json_and_utf8_remain_errors() {
    let data = body([r#""ok""#, "null", "null", "null"]);
    for end in 0..data.len() {
        assert!(decode(&data[..end]).is_err(), "prefix {end}");
    }
    let mut trailing = data.clone();
    trailing.push(0);
    assert!(decode(&trailing).is_err());
    for bad in ["null junk", "\u{00a0}null", "[", "[]", "not JSON"] {
        assert!(
            decode(&body([bad, "null", "null", "null"])).is_err(),
            "{bad}"
        );
    }
    let mut invalid = vec![0; 8];
    invalid.extend([1, 255]);
    assert!(decode(&invalid).is_err());
}
#[test]
fn old_sign_length_limit_and_other_packet_strictness_are_preserved() {
    let too_long = format!("{}null", " ".repeat(32764));
    assert!(decode(&body([&too_long, "null", "null", "null"])).is_err());
    let valid = format!("\"{}\"", "a".repeat(32765));
    assert!(decode(&body([&valid, "null", "null", "null"])).is_ok());
    let mut w = Writer::new();
    w.string("null");
    assert!(crate::adapter::read_legacy_component(&mut Reader::new(&w.data)).is_err());
    w.u8(0);
    assert!(State::new(1).clientbound(2, &w.data, &Cx).is_err());
}
