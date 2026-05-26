/// Red-black tree for CFS run queue — O(log n) insert, erase, min.
use crate::proc::id::Tid;

pub const MAX_RB_NODES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RbNodeId(u16);

impl RbNodeId {
    pub const INVALID: RbNodeId = RbNodeId(u16::MAX);
    pub const NONE: RbNodeId = RbNodeId(u16::MAX);

    pub fn new(id: u16) -> Self {
        Self(id)
    }

    pub fn as_usize(self) -> usize {
        self.0 as usize
    }

    pub fn is_valid(self) -> bool {
        self.0 != u16::MAX
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Color {
    Red,
    Black,
}

#[derive(Clone, Copy, Debug)]
struct RbNode {
    key: u64,
    tid: Tid,
    parent: RbNodeId,
    left: RbNodeId,
    right: RbNodeId,
    color: Color,
    in_use: bool,
}

impl RbNode {
    const fn empty() -> Self {
        Self {
            key: 0,
            tid: Tid::INVALID,
            parent: RbNodeId::INVALID,
            left: RbNodeId::INVALID,
            right: RbNodeId::INVALID,
            color: Color::Black,
            in_use: false,
        }
    }
}

pub struct RedBlackTree {
    nodes: [RbNode; MAX_RB_NODES],
    root: RbNodeId,
    free_head: RbNodeId,
}

impl RedBlackTree {
    pub const fn new() -> Self {
        Self {
            nodes: [RbNode::empty(); MAX_RB_NODES],
            root: RbNodeId::INVALID,
            free_head: RbNodeId::INVALID,
        }
    }

    pub fn init(&mut self) {
        for i in 0..MAX_RB_NODES {
            self.nodes[i].in_use = false;
            self.nodes[i].left = if i + 1 < MAX_RB_NODES {
                RbNodeId::new((i + 1) as u16)
            } else {
                RbNodeId::INVALID
            };
        }
        self.free_head = RbNodeId::new(0);
        self.root = RbNodeId::INVALID;
    }

    pub fn is_empty(&self) -> bool {
        !self.root.is_valid()
    }

    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|n| n.in_use).count()
    }

    fn alloc_node(&mut self) -> Option<RbNodeId> {
        if !self.free_head.is_valid() {
            return None;
        }
        let id = self.free_head;
        let next = self.nodes[id.as_usize()].left;
        self.nodes[id.as_usize()] = RbNode::empty();
        self.nodes[id.as_usize()].in_use = true;
        self.free_head = next;
        Some(id)
    }

    fn free_node(&mut self, id: RbNodeId) {
        let idx = id.as_usize();
        self.nodes[idx].in_use = false;
        self.nodes[idx].left = self.free_head;
        self.free_head = id;
    }

    pub fn insert(&mut self, key: u64, tid: Tid) -> Option<RbNodeId> {
        let node_id = self.alloc_node()?;
        {
            let node = &mut self.nodes[node_id.as_usize()];
            node.key = key;
            node.tid = tid;
            node.color = Color::Red;
            node.parent = RbNodeId::INVALID;
            node.left = RbNodeId::INVALID;
            node.right = RbNodeId::INVALID;
        }
        self.insert_node(node_id);
        Some(node_id)
    }

    fn insert_node(&mut self, node_id: RbNodeId) {
        if !self.root.is_valid() {
            self.root = node_id;
            self.nodes[node_id.as_usize()].color = Color::Black;
            return;
        }

        let key = self.nodes[node_id.as_usize()].key;
        let mut current = self.root;
        loop {
            let cur = &mut self.nodes[current.as_usize()];
            if key < cur.key {
                if !cur.left.is_valid() {
                    cur.left = node_id;
                    break;
                }
                current = cur.left;
            } else {
                if !cur.right.is_valid() {
                    cur.right = node_id;
                    break;
                }
                current = cur.right;
            }
        }
        self.nodes[node_id.as_usize()].parent = current;
        self.fix_insert(node_id);
    }

    pub fn remove(&mut self, node_id: RbNodeId) -> Option<Tid> {
        if !node_id.is_valid() || !self.nodes[node_id.as_usize()].in_use {
            return None;
        }
        let tid = self.nodes[node_id.as_usize()].tid;
        self.delete_node(node_id);
        self.free_node(node_id);
        Some(tid)
    }

    pub fn min(&self) -> Option<(RbNodeId, Tid, u64)> {
        if !self.root.is_valid() {
            return None;
        }
        let mut current = self.root;
        loop {
            let left = self.nodes[current.as_usize()].left;
            if left.is_valid() {
                current = left;
            } else {
                let node = &self.nodes[current.as_usize()];
                return Some((current, node.tid, node.key));
            }
        }
    }

    pub fn peek_min_key(&self) -> Option<u64> {
        self.min().map(|(_, _, k)| k)
    }

    fn fix_insert(&mut self, mut node_id: RbNodeId) {
        while node_id != self.root {
            let parent_id = self.nodes[node_id.as_usize()].parent;
            if !parent_id.is_valid() {
                break;
            }
            if self.nodes[parent_id.as_usize()].color != Color::Red {
                break;
            }
            let grandparent = self.nodes[parent_id.as_usize()].parent;
            if !grandparent.is_valid() {
                break;
            }
            let parent_is_left = self.nodes[grandparent.as_usize()].left == parent_id;
            let uncle_id = if parent_is_left {
                self.nodes[grandparent.as_usize()].right
            } else {
                self.nodes[grandparent.as_usize()].left
            };

            if uncle_id.is_valid() && self.nodes[uncle_id.as_usize()].color == Color::Red {
                self.nodes[parent_id.as_usize()].color = Color::Black;
                self.nodes[uncle_id.as_usize()].color = Color::Black;
                self.nodes[grandparent.as_usize()].color = Color::Red;
                node_id = grandparent;
                continue;
            }

            if parent_is_left {
                if self.nodes[parent_id.as_usize()].right == node_id {
                    self.rotate_left(parent_id);
                    node_id = parent_id;
                }
                self.nodes[node_id.as_usize()].parent = grandparent;
                self.rotate_right(grandparent);
                self.nodes[self.nodes[grandparent.as_usize()].parent.as_usize()].color = Color::Black;
                self.nodes[grandparent.as_usize()].color = Color::Red;
            } else {
                if self.nodes[parent_id.as_usize()].left == node_id {
                    self.rotate_right(parent_id);
                    node_id = parent_id;
                }
                self.nodes[node_id.as_usize()].parent = grandparent;
                self.rotate_left(grandparent);
                self.nodes[self.nodes[grandparent.as_usize()].parent.as_usize()].color = Color::Black;
                self.nodes[grandparent.as_usize()].color = Color::Red;
            }
            break;
        }
        self.nodes[self.root.as_usize()].color = Color::Black;
    }

    fn rotate_left(&mut self, x_id: RbNodeId) {
        let y_id = self.nodes[x_id.as_usize()].right;
        self.nodes[x_id.as_usize()].right = self.nodes[y_id.as_usize()].left;
        if self.nodes[y_id.as_usize()].left.is_valid() {
            self.nodes[self.nodes[y_id.as_usize()].left.as_usize()].parent = x_id;
        }
        self.nodes[y_id.as_usize()].parent = self.nodes[x_id.as_usize()].parent;
        let parent = self.nodes[x_id.as_usize()].parent;
        if !parent.is_valid() {
            self.root = y_id;
        } else if self.nodes[parent.as_usize()].left == x_id {
            self.nodes[parent.as_usize()].left = y_id;
        } else {
            self.nodes[parent.as_usize()].right = y_id;
        }
        self.nodes[y_id.as_usize()].left = x_id;
        self.nodes[x_id.as_usize()].parent = y_id;
    }

    fn rotate_right(&mut self, y_id: RbNodeId) {
        let x_id = self.nodes[y_id.as_usize()].left;
        self.nodes[y_id.as_usize()].left = self.nodes[x_id.as_usize()].right;
        if self.nodes[x_id.as_usize()].right.is_valid() {
            self.nodes[self.nodes[x_id.as_usize()].right.as_usize()].parent = y_id;
        }
        self.nodes[x_id.as_usize()].parent = self.nodes[y_id.as_usize()].parent;
        let parent = self.nodes[y_id.as_usize()].parent;
        if !parent.is_valid() {
            self.root = x_id;
        } else if self.nodes[parent.as_usize()].right == y_id {
            self.nodes[parent.as_usize()].right = x_id;
        } else {
            self.nodes[parent.as_usize()].left = x_id;
        }
        self.nodes[x_id.as_usize()].right = y_id;
        self.nodes[y_id.as_usize()].parent = x_id;
    }

    fn delete_node(&mut self, z_id: RbNodeId) {
        let (mut y_id, mut y_original_black) = {
            let z = &self.nodes[z_id.as_usize()];
            if !z.left.is_valid() || !z.right.is_valid() {
                (z_id, z.color == Color::Black)
            } else {
                let mut y = z.right;
                while self.nodes[y.as_usize()].left.is_valid() {
                    y = self.nodes[y.as_usize()].left;
                }
                (y, self.nodes[y.as_usize()].color == Color::Black)
            }
        };

        let x_id = if !self.nodes[y_id.as_usize()].left.is_valid() {
            self.nodes[y_id.as_usize()].right
        } else {
            self.nodes[y_id.as_usize()].left
        };

        if x_id.is_valid() {
            self.nodes[x_id.as_usize()].parent = self.nodes[y_id.as_usize()].parent;
        }
        let y_parent = self.nodes[y_id.as_usize()].parent;
        if !y_parent.is_valid() {
            self.root = x_id;
        } else if self.nodes[y_parent.as_usize()].left == y_id {
            self.nodes[y_parent.as_usize()].left = x_id;
        } else {
            self.nodes[y_parent.as_usize()].right = x_id;
        }

        if y_id != z_id {
            self.transplant(z_id, y_id);
            y_id = z_id;
        }

        if y_original_black {
            self.fix_delete(x_id);
        }
        let _ = y_id;
    }

    fn transplant(&mut self, u_id: RbNodeId, v_id: RbNodeId) {
        let u = self.nodes[u_id.as_usize()];
        let v_node = self.nodes[v_id.as_usize()];
        self.nodes[v_id.as_usize()] = u;
        self.nodes[v_id.as_usize()].left = v_node.left;
        self.nodes[v_id.as_usize()].right = v_node.right;
        self.nodes[v_id.as_usize()].color = v_node.color;
        self.nodes[v_id.as_usize()].tid = v_node.tid;
        self.nodes[v_id.as_usize()].key = v_node.key;
        if self.nodes[v_id.as_usize()].left.is_valid() {
            self.nodes[self.nodes[v_id.as_usize()].left.as_usize()].parent = v_id;
        }
        if self.nodes[v_id.as_usize()].right.is_valid() {
            self.nodes[self.nodes[v_id.as_usize()].right.as_usize()].parent = v_id;
        }
        let parent = u.parent;
        if !parent.is_valid() {
            self.root = v_id;
        } else if self.nodes[parent.as_usize()].left == u_id {
            self.nodes[parent.as_usize()].left = v_id;
        } else {
            self.nodes[parent.as_usize()].right = v_id;
        }
        self.nodes[v_id.as_usize()].parent = parent;
    }

    fn node_color(&self, id: RbNodeId) -> Color {
        if id.is_valid() {
            self.nodes[id.as_usize()].color
        } else {
            Color::Black
        }
    }

    fn fix_delete(&mut self, mut x_id: RbNodeId) {
        while x_id != self.root && (!x_id.is_valid() || self.node_color(x_id) == Color::Black) {
            if !x_id.is_valid() {
                break;
            }
            let parent = self.nodes[x_id.as_usize()].parent;
            if !parent.is_valid() {
                break;
            }
            if self.nodes[parent.as_usize()].left == x_id {
                let mut w_id = self.nodes[parent.as_usize()].right;
                if w_id.is_valid() && self.nodes[w_id.as_usize()].color == Color::Red {
                    self.nodes[w_id.as_usize()].color = Color::Black;
                    self.nodes[parent.as_usize()].color = Color::Red;
                    self.rotate_left(parent);
                    w_id = self.nodes[parent.as_usize()].right;
                }
                if w_id.is_valid()
                    && self.node_color(self.nodes[w_id.as_usize()].left) == Color::Black
                    && self.node_color(self.nodes[w_id.as_usize()].right) == Color::Black
                {
                    self.nodes[w_id.as_usize()].color = Color::Red;
                    x_id = parent;
                } else if w_id.is_valid() {
                    if self.node_color(self.nodes[w_id.as_usize()].right) == Color::Black {
                        let left = self.nodes[w_id.as_usize()].left;
                        if left.is_valid() {
                            self.nodes[left.as_usize()].color = Color::Black;
                        }
                        self.nodes[w_id.as_usize()].color = Color::Red;
                        self.rotate_right(w_id);
                        w_id = self.nodes[parent.as_usize()].right;
                    }
                    if w_id.is_valid() {
                        self.nodes[w_id.as_usize()].color = self.nodes[parent.as_usize()].color;
                        self.nodes[parent.as_usize()].color = Color::Black;
                        let right = self.nodes[w_id.as_usize()].right;
                        if right.is_valid() {
                            self.nodes[right.as_usize()].color = Color::Black;
                        }
                        self.rotate_left(parent);
                        x_id = self.root;
                    }
                } else {
                    break;
                }
            } else {
                let mut w_id = self.nodes[parent.as_usize()].left;
                if w_id.is_valid() && self.nodes[w_id.as_usize()].color == Color::Red {
                    self.nodes[w_id.as_usize()].color = Color::Black;
                    self.nodes[parent.as_usize()].color = Color::Red;
                    self.rotate_right(parent);
                    w_id = self.nodes[parent.as_usize()].left;
                }
                if w_id.is_valid()
                    && self.node_color(self.nodes[w_id.as_usize()].right) == Color::Black
                    && self.node_color(self.nodes[w_id.as_usize()].left) == Color::Black
                {
                    self.nodes[w_id.as_usize()].color = Color::Red;
                    x_id = parent;
                } else if w_id.is_valid() {
                    if self.node_color(self.nodes[w_id.as_usize()].left) == Color::Black {
                        let right = self.nodes[w_id.as_usize()].right;
                        if right.is_valid() {
                            self.nodes[right.as_usize()].color = Color::Black;
                        }
                        self.nodes[w_id.as_usize()].color = Color::Red;
                        self.rotate_left(w_id);
                        w_id = self.nodes[parent.as_usize()].left;
                    }
                    if w_id.is_valid() {
                        self.nodes[w_id.as_usize()].color = self.nodes[parent.as_usize()].color;
                        self.nodes[parent.as_usize()].color = Color::Black;
                        let left = self.nodes[w_id.as_usize()].left;
                        if left.is_valid() {
                            self.nodes[left.as_usize()].color = Color::Black;
                        }
                        self.rotate_right(parent);
                        x_id = self.root;
                    }
                } else {
                    break;
                }
            }
        }
        if x_id.is_valid() {
            self.nodes[x_id.as_usize()].color = Color::Black;
        }
    }
}
