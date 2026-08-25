//! HAL tree model: six roots (Components, Pins, Parameters, Signals,
//! Functions, Threads). Pin/param/signal names nest on every '.'.

use crate::hal::{HalSession, HalType};
use regex::Regex;

#[derive(Clone, Debug)]
pub struct TreeNode {
    /// Display segment (last path component).
    pub name: String,
    /// Full node id, e.g. "pin", "pin+axis", "pin+axis.0.pos".
    pub path: String,
    pub kind: HalType,
    pub leaf: bool,
    pub children: Vec<TreeNode>,
    pub expanded: bool,
}

impl TreeNode {
    pub fn is_branch(&self) -> bool {
        !self.children.is_empty()
    }
}

#[derive(Debug)]
pub struct HalTree {
    pub roots: Vec<TreeNode>,
    pub selected: String,
    pub filter: String,
    pub full_path: bool,
}

impl HalTree {
    pub fn new() -> Self {
        HalTree {
            roots: Vec::new(),
            selected: String::new(),
            filter: String::new(),
            full_path: false,
        }
    }

    /// Rebuild the tree from `hal list` data. Expanded nodes and the
    /// selection survive the rebuild when they still exist.
    pub fn rebuild(&mut self, hal: &HalSession) {
        let open: Vec<String> = visible(&self.roots)
            .iter()
            .filter(|(_, _, b)| *b)
            .map(|(p, _, _)| p.clone())
            .collect();
        let mut roots = Vec::new();
        for t in HalType::ALL {
            let mut root = TreeNode {
                name: t.title().to_string(),
                path: t.kw().to_string(),
                kind: t,
                leaf: false,
                children: Vec::new(),
                expanded: true,
            };
            if let Ok(re) = Regex::new(&self.filter) {
                for item in hal.list(t) {
                    if !filter_match(&re, &item, self.full_path) {
                        continue;
                    }
                    match t {
                        HalType::Pin | HalType::Param | HalType::Sig => {
                            insert_dotted(&mut root, &item);
                        }
                        _ => {
                            root.children.push(TreeNode {
                                name: item.clone(),
                                path: format!("{}+{}", t.kw(), item),
                                kind: t,
                                leaf: true,
                                children: Vec::new(),
                                expanded: false,
                            });
                        }
                    }
                }
            }
            roots.push(root);
        }
        self.roots = roots;
        if !self.filter.is_empty() {
            // filter active: reveal every surviving branch so matches are
            // immediately visible (mirrors halshow's openTreePath-on-hit)
            set_all_expanded(&mut self.roots, true);
        } else {
            // no filter: restore the previously expanded state
            let reopen = |path: &str, roots: &mut Vec<TreeNode>| {
                let mut cur = roots;
                let mut acc = String::new();
                for seg in path.split(['+', '.']) {
                    acc = if acc.is_empty() {
                        seg.to_string()
                    } else {
                        format!("{}.{}", acc, seg)
                    };
                    let idx = cur.iter().position(|n| n.path == acc);
                    match idx {
                        Some(i) => {
                            cur[i].expanded = true;
                            cur = &mut cur[i].children;
                        }
                        None => break,
                    }
                }
            };
            for p in &open {
                reopen(p, &mut self.roots);
            }
        }
        // keep selection if it still exists
        if !self.path_exists(&self.selected) {
            self.selected = visible(&self.roots)
                .first()
                .map(|(p, _, _)| p.clone())
                .unwrap_or_default();
        }
    }

    fn path_exists(&self, path: &str) -> bool {
        visible(&self.roots).iter().any(|(p, _, _)| p == path)
    }

    pub fn selected_node(&self) -> Option<&TreeNode> {
        find_node(&self.roots, &self.selected)
    }

    pub fn expand_all(&mut self) {
        set_all_expanded(&mut self.roots, true);
    }

    pub fn collapse_all(&mut self) {
        set_all_expanded(&mut self.roots, false);
    }

    pub fn expand_kind(&mut self, kind: HalType) {
        set_kind_expanded(&mut self.roots, kind, true);
    }

    pub fn collapse_kind(&mut self, kind: HalType) {
        set_kind_expanded(&mut self.roots, kind, false);
    }

    /// Expand the path to `path` and select it.
    pub fn reveal(&mut self, path: &str) {
        let mut cur = &mut self.roots;
        let mut acc = String::new();
        for seg in path.split(['+', '.']) {
            if acc.is_empty() {
                acc = seg.to_string();
            } else {
                acc = format!("{}.{}", acc, seg);
            }
            let idx = cur.iter().position(|n| n.path == acc);
            match idx {
                Some(i) => {
                    cur[i].expanded = true;
                    cur = &mut cur[i].children;
                }
                None => return,
            }
        }
        self.selected = path.to_string();
    }
}

/// Insert a dotted name ("axis.0.pos") as a nested chain under `root`.
fn insert_dotted(root: &mut TreeNode, item: &str) {
    let kind = root.kind;
    let mut cur: &mut TreeNode = root;
    let segs: Vec<&str> = item.split('.').collect();
    let mut acc = String::new();
    for (i, seg) in segs.iter().enumerate() {
        if acc.is_empty() {
            acc = format!("{}+{}", kind.kw(), seg);
        } else {
            acc = format!("{}.{}", acc, seg);
        }
        let leaf = i == segs.len() - 1;
        // find existing child
        let pos = cur.children.iter().position(|c| c.path == acc);
        if let Some(pos) = pos {
            cur = &mut cur.children[pos];
        } else {
            cur.children.push(TreeNode {
                name: seg.to_string(),
                path: acc.clone(),
                kind,
                leaf,
                children: Vec::new(),
                expanded: false,
            });
            let last = cur.children.len() - 1;
            cur = &mut cur.children[last];
        }
    }
}

fn filter_match(re: &Regex, item: &str, full_path: bool) -> bool {
    if full_path {
        re.is_match(item)
    } else {
        item.split('.').any(|seg| re.is_match(seg))
    }
}

/// Depth-first flatten of visible (expanded) nodes.
/// Returns (path, depth, is_branch) in render order.
pub fn visible(roots: &[TreeNode]) -> Vec<(String, usize, bool)> {
    let mut out = Vec::new();
    fn walk(nodes: &[TreeNode], depth: usize, out: &mut Vec<(String, usize, bool)>) {
        for n in nodes {
            out.push((n.path.clone(), depth, n.is_branch()));
            if n.expanded {
                walk(&n.children, depth + 1, out);
            }
        }
    }
    walk(roots, 0, &mut out);
    out
}

pub fn find_node<'a>(nodes: &'a [TreeNode], path: &str) -> Option<&'a TreeNode> {
    for n in nodes {
        if n.path == path {
            return Some(n);
        }
        if let Some(found) = find_node(&n.children, path) {
            return Some(found);
        }
    }
    None
}

fn set_all_expanded(nodes: &mut [TreeNode], value: bool) {
    for n in nodes {
        n.expanded = value;
        set_all_expanded(&mut n.children, value);
    }
}

fn set_kind_expanded(nodes: &mut [TreeNode], kind: HalType, value: bool) {
    for n in nodes {
        if n.kind == kind {
            n.expanded = value;
        }
        set_kind_expanded(&mut n.children, kind, value);
    }
}

/// Full HAL item name of a node path ("pin+axis.0.pos" -> "axis.0.pos").
pub fn full_name(path: &str) -> Option<&str> {
    path.split_once('+').map(|(_, rest)| rest)
}

/// All watchable leaves below a node path.
pub fn collect_leaves(nodes: &[TreeNode], path: &str) -> Vec<(HalType, String)> {
    let mut out = Vec::new();
    if let Some(node) = find_node(nodes, path) {
        if node.leaf && node.kind.watchable() {
            if let Some(name) = full_name(&node.path) {
                out.push((node.kind, name.to_string()));
            }
        }
        let mut stack: Vec<&TreeNode> = node.children.iter().collect();
        while let Some(n) = stack.pop() {
            if n.leaf && n.kind.watchable() {
                if let Some(name) = full_name(&n.path) {
                    out.push((n.kind, name.to_string()));
                }
            }
            stack.extend(n.children.iter());
        }
    }
    out
}

/// Strip the last path component ("pin+axis.0.pos" -> "pin+axis.0").
pub fn parent_path(path: &str) -> Option<String> {
    match (path.rfind('.'), path.rfind('+')) {
        (Some(d), Some(p)) => {
            let cut = d.max(p);
            Some(path[..cut].to_string())
        }
        (Some(d), None) => Some(path[..d].to_string()),
        (None, Some(p)) => Some(path[..p].to_string()),
        (None, None) => None,
    }
}
