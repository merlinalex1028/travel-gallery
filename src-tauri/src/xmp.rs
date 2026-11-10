//! Standalone export metadata only. Never opens or edits source sidecars.
use crate::error::Result;
use rusqlite::Connection;
use serde_json::Value;

fn valid_xml(c: char) -> bool {
    matches!(c as u32, 0x9|0xA|0xD|0x20..=0xD7FF|0xE000..=0xFFFD|0x10000..=0x10FFFF)
}
fn text(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            c if valid_xml(c) => out.push(c),
            _ => out.push('\u{fffd}'),
        }
    }
    out
}
pub struct Packet {
    pub content: String,
    pub replaced_characters: usize,
}
pub fn packet(c: &Connection, asset: &Value) -> Result<Packet> {
    let mut statement=c.prepare("SELECT t.name FROM tags t JOIN asset_tags at ON at.tag_id=t.id WHERE at.asset_id=? ORDER BY t.name,t.id")?;
    let tags = statement
        .query_map([asset["id"].as_str()], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let note = asset["review"]["note"].as_str().unwrap_or("");
    let replaced = tags
        .iter()
        .map(|s| s.chars().filter(|c| !valid_xml(*c)).count())
        .sum::<usize>()
        + note.chars().filter(|c| !valid_xml(*c)).count();
    // Keep star rating and pick state independent, even when a rejected photo has stars.
    let rating = asset["review"]["rating"].as_i64().unwrap_or(0);
    let pick = asset["review"]["pickState"]
        .as_str()
        .unwrap_or("unreviewed");
    let favorite = if asset["review"]["isFavorite"] == true {
        "True"
    } else {
        "False"
    };
    let subjects = tags
        .iter()
        .map(|tag| format!("<rdf:li>{}</rdf:li>", text(tag)))
        .collect::<String>();
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Travel Gallery 0.1.0">
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:tg="urn:travel-gallery:metadata:1.0">
<xmp:CreatorTool>Travel Gallery 0.1.0</xmp:CreatorTool>
<xmp:Rating>{rating}</xmp:Rating>
<tg:PickState>{pick}</tg:PickState>
<tg:IsFavorite>{favorite}</tg:IsFavorite>
<dc:subject><rdf:Bag>{subjects}</rdf:Bag></dc:subject>
<dc:description><rdf:Alt><rdf:li xml:lang="x-default">{note}</rdf:li></rdf:Alt></dc:description>
</rdf:Description>
</rdf:RDF>
</x:xmpmeta>
"#,
        pick = text(pick),
        note = text(note)
    );
    Ok(Packet {
        content,
        replaced_characters: replaced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query;
    use quick_xml::{events::Event, Reader};
    #[test]
    fn arbitrary_unicode_is_xml_text_and_rating_never_conflates_rejection() {
        let temp = tempfile::tempdir().unwrap();
        let cat =
            crate::catalog::Catalog::new(&temp.path().join("data"), &temp.path().join("cache"))
                .unwrap();
        let c = cat.connection().unwrap();
        c.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES('s','s','/unused',0,0)",[]).unwrap();
        c.execute("INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,indexed_at,updated_at) VALUES('a','s','a.jpg','a.jpg','jpg','image',0,0,0)",[]).unwrap();
        c.execute(
            "INSERT INTO tags(id,name,created_at) VALUES('t',?,0)",
            ["山<&\"'📷"],
        )
        .unwrap();
        c.execute("INSERT INTO asset_tags VALUES('a','t')", [])
            .unwrap();
        c.execute("UPDATE asset_reviews SET pick_state='reject',rating=4,is_favorite=1,note=? WHERE asset_id='a'",["</rdf:li><evil/> & 🏔️\r\n\t\u{0}\u{ffff}"]).unwrap();
        let packet = packet(&c, &query::asset(&c, "a").unwrap()).unwrap();
        assert_eq!(packet.replaced_characters, 2);
        let mut reader = Reader::from_str(&packet.content);
        let mut values = vec![];
        loop {
            match reader.read_event().unwrap() {
                Event::Start(start)
                    if ["xmp:Rating", "tg:PickState", "tg:IsFavorite", "rdf:li"]
                        .contains(&start.name().as_ref()) =>
                {
                    let value = reader.read_text(start.name()).unwrap();
                    values.push(quick_xml::escape::unescape(&value).unwrap().into_owned());
                }
                Event::Empty(start) => assert_ne!(start.name().as_ref(), "evil"),
                Event::Eof => break,
                _ => {}
            }
        }
        assert_eq!(
            values,
            vec![
                "4",
                "reject",
                "True",
                "山<&\"'📷",
                "</rdf:li><evil/> & 🏔️\r\n\t��"
            ]
        );
    }
}
