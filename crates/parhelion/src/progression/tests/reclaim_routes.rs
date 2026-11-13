use super::*;

fn array(data: &mut Vec<u8>, field: usize, count: usize, class: u32, stride: usize) -> usize {
    let header = data.len();
    data.resize(header + 16 + count * stride, 0);
    write_u64(data, field, count as u64).unwrap();
    write_relative_pointer(data, field + 8, header).unwrap();
    write_u64(data, header, count as u64).unwrap();
    write_u32(data, header + 8, class).unwrap();
    header + 16
}

fn scene() -> (Vec<u8>, usize, usize) {
    let mut nodes = vec![0; 24];
    let rows = array(
        &mut nodes,
        8,
        4,
        PRESENTATION_NODE_DEFINITION_ROW_CLASS,
        PRESENTATION_NODE_ROW_SIZE,
    );
    for (node, hash) in [(0, 0xE1EA_9713), (2, BADGES_ROOT_NODE_HASH)] {
        write_u32(
            &mut nodes,
            rows + node * PRESENTATION_NODE_ROW_SIZE + PRESENTATION_NODE_HASH_OFFSET,
            hash,
        )
        .unwrap();
    }
    for (root, child) in [(0, 1), (2, 3)] {
        let at = array(
            &mut nodes,
            rows + root * PRESENTATION_NODE_ROW_SIZE + PRESENTATION_NODE_CHILD_NODES_OFFSET,
            1,
            PRESENTATION_NODE_CHILD_NODE_ROW_CLASS,
            PRESENTATION_NODE_CHILD_NODE_ROW_SIZE,
        );
        write_u16(&mut nodes, at, child).unwrap();
    }
    let mut category_child = 0;
    for page in [1, 3] {
        let at = array(
            &mut nodes,
            rows + page * PRESENTATION_NODE_ROW_SIZE + PRESENTATION_NODE_COLLECTIBLES_OFFSET,
            1,
            PRESENTATION_NODE_COLLECTIBLE_ROW_CLASS,
            PRESENTATION_NODE_COLLECTIBLE_ROW_SIZE,
        );
        write_u16(&mut nodes, at, 7).unwrap();
        if page == 1 {
            category_child = at;
        }
    }
    (nodes, rows, category_child)
}

fn entry(page: u16) -> ProjectAuthoredRow {
    ProjectAuthoredRow {
        donor_collectible_index: 1,
        authored_collectible_index: 7,
        pages: crate::collection::CollectionPages::one(page),
        classes: crate::collection::Classes::ALL,
        source_acquired_flag: 2,
        authored_unlock_index: 3,
        count_selection: Default::default(),
    }
}

#[test]
fn reclaim_requires_category_membership_beyond_a_badge() {
    let (nodes, _, _) = scene();
    validate_reclaim_pages(&nodes, &[entry(1)]).unwrap();
    let error = validate_reclaim_pages(&nodes, &[entry(3)]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("badge membership alone only allows inspection")
    );
}

#[test]
fn reclaim_rejects_unlisted_or_unreachable_category_entries() {
    let (mut nodes, rows, child) = scene();
    write_u16(&mut nodes, child, 8).unwrap();
    assert!(validate_reclaim_pages(&nodes, &[entry(1)]).is_err());
    write_u16(&mut nodes, child, 7).unwrap();
    write_u64(&mut nodes, rows + PRESENTATION_NODE_CHILD_NODES_OFFSET, 0).unwrap();
    assert!(validate_reclaim_pages(&nodes, &[entry(1)]).is_err());
}
