//! `Edit::Append`: one more item at the end of a list, written the way the list is.

use super::{
    Kind, Node, Outcome, PathSeg, SHARED, SpliceError, line_end, line_start, locate, skip,
};

pub(super) fn append(
    t: &str,
    root: Option<&Node>,
    parent: &[PathSeg],
    value: &str,
    nl: &str,
) -> Result<Outcome, SpliceError> {
    let (n, _, inside) = locate(root, parent)?;
    if n.anchored || inside {
        return skip(parent, SHARED);
    }
    let Kind::Seq { flow, items } = &n.kind else {
        return skip(
            parent,
            "not a list (an empty or mapping value): edit it in the file",
        );
    };
    let (range, with) = match (items.last(), flow) {
        (Some(last), true) => (last.end..last.end, format!(", {value}")),
        (None, true) => (n.start..n.end, format!("[ {value} ]")),
        (Some(last), false) => {
            // new line after the last item, under the first item's dash
            let first = &items[0];
            let ls = line_start(t, first.start);
            if t[ls..first.start].trim() != "-" {
                return skip(
                    parent,
                    "an item shares its line with something else: edit it in the file",
                );
            }
            let at = line_end(t, last.end);
            let dash = &t[ls..first.start];
            // `-` with nothing after it (a null first item) has no space to copy
            let gap = if dash.ends_with(char::is_whitespace) {
                ""
            } else {
                " "
            };
            (at..at, format!("{nl}{dash}{gap}{value}"))
        }
        (None, false) => return skip(parent, "an empty list: edit it in the file"),
    };
    Ok(Outcome::Replace(range, with, Vec::new()))
}
