//! `Edit::Swap`: two items of one list trade places, each with the text it was written with.

use super::{Kind, Node, Outcome, PathSeg, SHARED, SpliceError, locate, skip};

pub(super) fn swap(
    text: &str,
    root: Option<&Node>,
    one: &[PathSeg],
    other: &[PathSeg],
) -> Result<Outcome, SpliceError> {
    let (na, _, in_a) = locate(root, one)?;
    let (nb, _, in_b) = locate(root, other)?;
    // siblings of one list: the same parent path, two different indexes
    let same_list = match (one.split_last(), other.split_last()) {
        (Some((PathSeg::Index(i), pa)), Some((PathSeg::Index(j), pb))) => pa == pb && i != j,
        _ => false,
    };
    if !same_list {
        return skip(one, "can swap only two different items of one list");
    }
    let parent = &one[..one.len() - 1];
    if let Ok((p, _, _)) = locate(root, parent)
        && !matches!(p.kind, Kind::Seq { .. })
    {
        return skip(one, "not a list");
    }
    if na.anchored || nb.anchored || in_a || in_b {
        return skip(one, SHARED);
    }
    let (first, second) = if na.start <= nb.start {
        (na, nb)
    } else {
        (nb, na)
    };
    // each item's text goes where the other one was; whatever sat between them (comments, the dash
    // and the line break of the second) stays where it is
    let with = format!(
        "{}{}{}",
        &text[second.start..second.end],
        &text[first.end..second.start],
        &text[first.start..first.end]
    );
    Ok(Outcome::Replace(first.start..second.end, with, Vec::new()))
}
