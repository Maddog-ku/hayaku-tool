//! 對應 C# `Core/MyTaskTree.cs`：把我的任務依專案分組，再照上下層排成樹（上層不在清單裡的也補上當脈絡）。

use std::collections::HashMap;

use hb_api::TaskSummary;

use crate::ci_set::{eq_ic, fold};

/// 我的任務樹的一個節點；task 為 None 表示只是上層脈絡（不在這次的清單裡）。
#[derive(Debug, Clone, PartialEq)]
pub struct MyTaskNode {
    pub key: String,
    pub title: String,
    pub task: Option<TaskSummary>,
    pub children: Vec<MyTaskNode>,
}

/// 一個專案底下的我的任務。
#[derive(Debug, Clone, PartialEq)]
pub struct MyTaskProject {
    pub project: String,
    pub roots: Vec<MyTaskNode>,
    pub count: usize,
}

pub struct MyTaskTree;

struct Arena {
    key: String,
    title: String,
    task: Option<TaskSummary>,
    children: Vec<usize>,
}

struct Group {
    project: String,
    roots: Vec<usize>,
    nodes: Vec<Arena>,
    index: HashMap<String, usize>,
    count: usize,
}

impl MyTaskTree {
    /// 專案與兄弟節點的順序照清單第一次出現的順序，所以排序方式照樣有效。
    pub fn build<'a>(tasks: impl IntoIterator<Item = &'a TaskSummary>) -> Vec<MyTaskProject> {
        let mut groups: Vec<Group> = Vec::new();
        for t in tasks {
            let gi = match groups.iter().position(|g| g.project == t.project) {
                Some(i) => i,
                None => {
                    groups.push(Group {
                        project: t.project.clone(),
                        roots: Vec::new(),
                        nodes: Vec::new(),
                        index: HashMap::new(),
                        count: 0,
                    });
                    groups.len() - 1
                }
            };
            let group = &mut groups[gi];
            group.count += 1;

            let mut parent: Option<usize> = None; // None＝放在最上層
            let chain = t
                .ancestors
                .iter()
                .flatten()
                .map(|(k, title)| (k.as_str(), title.as_str()))
                .chain(std::iter::once((t.key.as_str(), t.title.as_str())));
            for (key, title) in chain {
                let node = match group.index.get(&fold(key)) {
                    Some(&n) => n,
                    None => {
                        let n = group.nodes.len();
                        group.nodes.push(Arena {
                            key: key.to_owned(),
                            title: title.to_owned(),
                            task: None,
                            children: Vec::new(),
                        });
                        group.index.insert(fold(key), n);
                        match parent {
                            None => group.roots.push(n),
                            Some(p) => group.nodes[p].children.push(n),
                        }
                        n
                    }
                };
                if eq_ic(key, &t.key) {
                    group.nodes[node].task = Some(t.clone());
                }
                parent = Some(node);
            }
        }
        groups
            .into_iter()
            .map(|g| {
                let roots = g.roots.iter().map(|&n| to_node(&g.nodes, n, 0)).collect();
                MyTaskProject { project: g.project, roots, count: g.count }
            })
            .collect()
    }
}

fn to_node(nodes: &[Arena], n: usize, depth: usize) -> MyTaskNode {
    let a = &nodes[n];
    // 上下層資料有環時（不該發生）不無限展開
    let children =
        if depth > 64 { Vec::new() } else { a.children.iter().map(|&c| to_node(nodes, c, depth + 1)).collect() };
    MyTaskNode { key: a.key.clone(), title: a.title.clone(), task: a.task.clone(), children }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 對應 MyTaskTreeTests

    fn t(key: &str, project: &str, ancestors: &[(&str, &str)]) -> TaskSummary {
        let mut s =
            TaskSummary::new(key, format!("{key}名稱"), project, "處理中", "IN_PROGRESS", "無", None, None, false, "");
        s.ancestors = Some(ancestors.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect());
        s
    }

    fn flat(nodes: &[MyTaskNode], depth: usize) -> Vec<String> {
        nodes
            .iter()
            .flat_map(|n| {
                std::iter::once(format!(
                    "{}{}{}",
                    "-".repeat(depth),
                    n.key,
                    if n.task.is_none() { "(上層)" } else { "" }
                ))
                .chain(flat(&n.children, depth + 1))
            })
            .collect()
    }

    #[test]
    fn build_依專案分組並補上不在清單裡的上層() {
        let tasks = [
            t("F2P-15", "F2P", &[("F2P-5", "第一階段"), ("F2P-14", "機密列印")]),
            t("EG-7", "EG", &[]),
            t("F2P-14", "F2P", &[("F2P-5", "第一階段")]),
            t("F2P-6", "F2P", &[("F2P-5", "第一階段")]),
            t("EG-8", "EG", &[("EG-7", "")]),
        ];
        let groups = MyTaskTree::build(&tasks);
        assert_eq!(vec!["F2P", "EG"], groups.iter().map(|g| g.project.as_str()).collect::<Vec<_>>());
        assert_eq!(vec!["F2P-5(上層)", "-F2P-14", "--F2P-15", "-F2P-6"], flat(&groups[0].roots, 0));
        assert_eq!(3, groups[0].count);
        assert_eq!(vec!["EG-7", "-EG-8"], flat(&groups[1].roots, 0));
    }
}
