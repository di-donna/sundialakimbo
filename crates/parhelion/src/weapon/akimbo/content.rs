//! Native C9D8C0 content selection, shared by compiler and visual assessment.
use crate::tag_payload::{read_u32, read_u64};
use crate::{AuthoringResult, error::invalid, weapon::behavior::Content};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Selection {
    /// u32::MAX names the embedded default; ordinary values name native rows.
    pub index: u32,
    pub block: usize,
}

pub(crate) fn select(content: &Content, group: u32) -> AuthoringResult<Selection> {
    let definition = usize::try_from(read_u64(&content.owner, content.resource + 8)?)
        .map_err(|_| invalid("Akimbo content definition overflow"))?;
    // Preserve the proven forward-array contract. The native count-zero path
    // never dereferences its unused relative array pointer.
    if read_u64(&content.owner, definition + 0x240)? > 0
        && read_u64(&content.owner, definition + 0x248)? > i64::MAX as u64
    {
        return Err(invalid(
            "Akimbo content variants require the native forward array layout",
        ));
    }
    let native = crate::weapon::ammo::property_offsets(&content.owner, definition)?;
    if native != content.blocks {
        return Err(invalid(
            "Akimbo content blocks differ from their native layout",
        ));
    }
    let mut selected = None;
    for (index, &block) in native.iter().skip(1).enumerate() {
        if read_u32(&content.owner, block + 0x10)? == group
            && selected
                .replace(Selection {
                    index: index as u32,
                    block,
                })
                .is_some()
        {
            return Err(invalid("Akimbo content selection is ambiguous"));
        }
    }
    // No matching variant is a native default route, even when the embedded
    // block's own group field differs. Duplicate matching variants never fall back.
    Ok(selected.unwrap_or(Selection {
        index: u32::MAX,
        block: native[0],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_payload::{write_u32, write_u64};

    fn fixture() -> Content {
        let mut content = Content {
            owner_tag: 0x8152a37a,
            owner: vec![0; 0x800],
            resource: 0x80,
            blocks: vec![0x180, 0x400, 0x5c0],
        };
        write_u64(&mut content.owner, 0x88, 0x100).unwrap();
        write_u64(&mut content.owner, 0x340, 2).unwrap();
        write_u64(&mut content.owner, 0x348, 0xa8).unwrap();
        write_u64(&mut content.owner, 0x3f0, 2).unwrap();
        write_u32(&mut content.owner, 0x3f8, 0x80803acf).unwrap();
        for (i, &block) in content.blocks.iter().enumerate() {
            write_u32(&mut content.owner, block + 0x10, 100 + i as u32).unwrap();
        }
        content
    }

    #[test]
    fn akimbo_content_default_and_indexed_routes_share_one_selector() {
        let content = fixture();
        assert_eq!(
            select(&content, 101).unwrap(),
            Selection {
                index: 0,
                block: 0x400
            }
        );
        assert_eq!(
            select(&content, 102).unwrap(),
            Selection {
                index: 1,
                block: 0x5c0
            }
        );
        for group in [0, 100, 103, u32::MAX] {
            assert_eq!(
                select(&content, group).unwrap(),
                Selection {
                    index: u32::MAX,
                    block: 0x180
                }
            );
        }
    }

    #[test]
    fn akimbo_content_default_never_hides_duplicate_or_malformed_rows() {
        let mut content = fixture();
        write_u32(&mut content.owner, 0x5d0, 101).unwrap();
        assert!(
            select(&content, 101)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        for (at, value) in [(0x340, 3), (0x3f0, 3), (0x3f8, 0), (0x348, u64::MAX)] {
            let mut content = fixture();
            write_u64(&mut content.owner, at, value).unwrap();
            assert!(select(&content, 999).is_err(), "malformed field {at:X}");
        }
        let mut content = fixture();
        content.blocks[0] = 0x80;
        assert!(select(&content, 999).is_err());
    }

    #[test]
    fn akimbo_content_empty_array_selects_bounded_embedded_default() {
        let mut content = fixture();
        content.blocks.truncate(1);
        write_u64(&mut content.owner, 0x340, 0).unwrap();
        write_u64(&mut content.owner, 0x348, u64::MAX).unwrap();
        content.owner.truncate(0x370);
        assert_eq!(
            select(&content, 999).unwrap(),
            Selection {
                index: u32::MAX,
                block: 0x180
            }
        );
        content.owner.pop();
        assert!(select(&content, 999).is_err());
    }
}
