//! Identifier-based 754 argument serializers converted before the modern reader.
use super::*;

pub(super) fn read(r: &mut Reader<'_>) -> Result<play::Commands> {
    let count = r.count(65536)?;
    if count == 0 {
        return Err(Error::new("empty old command tree"));
    }
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        let flags = r.u8()?;
        let kind = flags & 3;
        if flags & !31 != 0 || kind == 3 || flags & 16 != 0 && kind != 2 {
            return Err(Error::new("invalid old command flags"));
        }
        let n = r.count(count)?;
        let children = (0..n).map(|_| r.varint()).collect::<Result<Vec<_>>>()?;
        let redirect = if flags & 8 != 0 {
            Some(r.varint()?)
        } else {
            None
        };
        let name = if kind != 0 {
            r.string(32767)?
        } else {
            String::new()
        };
        let mut suggestions = None;
        let parser = if kind == 2 {
            let source = r.string(32767)?;
            let (target, registry) = match source.as_str() {
                "minecraft:color" => ("minecraft:team_color", None),
                "minecraft:entity_summon" => ("minecraft:resource", Some("minecraft:entity_type")),
                "minecraft:item_enchantment" => {
                    ("minecraft:resource", Some("minecraft:enchantment"))
                }
                "minecraft:mob_effect" => ("minecraft:resource", Some("minecraft:mob_effect")),
                name => (name, None),
            };
            let id = play::ARGUMENT_TYPES
                .iter()
                .position(|name| *name == target)
                .ok_or_else(|| Error::new("unknown old command argument parser"))?
                as i32;
            let properties = if let Some(registry) = registry {
                let mut w = Writer::new();
                w.string(registry);
                w.data
            } else if target == "minecraft:time" {
                vec![0; 4]
            } else {
                let start = r.position();
                play::ArgumentProperties::read(r, id)?;
                r.since(start).to_vec()
            };
            if flags & 16 != 0 {
                suggestions = Some(r.string(32767)?);
            }
            Some((id, properties))
        } else {
            None
        };
        if children
            .iter()
            .chain(redirect.iter())
            .any(|id| *id < 0 || *id as usize >= count)
        {
            return Err(Error::new("old command reference outside tree"));
        }
        nodes.push(play::CommandNode {
            kind,
            executable: flags & 4 != 0,
            restricted: false,
            children,
            redirect,
            name,
            parser,
            suggestions,
        });
    }
    let root = r.varint()?;
    if root < 0 || nodes.get(root as usize).is_none_or(|node| node.kind != 0) {
        return Err(Error::new("invalid old command root"));
    }
    Ok(play::Commands { nodes, root })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn property_free_old_time_retains_the_following_node_boundary() {
        let mut w = Writer::new();
        w.varint(3);
        w.u8(0);
        w.varint(1);
        w.varint(1);
        w.u8(2);
        w.varint(1);
        w.varint(2);
        w.string("duration");
        w.string("minecraft:time");
        w.u8(5);
        w.varint(0);
        w.string("after");
        w.varint(0);
        let mut r = Reader::new(&w.data);
        let tree = read(&mut r).unwrap();
        finish(&r).unwrap();
        assert_eq!(tree.nodes[1].parser, Some((43, vec![0, 0, 0, 0])));
        assert_eq!(tree.nodes[2].name, "after");
    }
}
