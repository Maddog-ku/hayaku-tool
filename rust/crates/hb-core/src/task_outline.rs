//! 對應 C# `Core/TaskOutline.cs`：把貼上的清單變成任務樹。
//! 節點型別 `OutlineNode` 在 hb-api（`HayakuApi::create_tasks` 要用），這裡直接用。

pub use hb_api::OutlineNode;

/// 把貼上的清單變成任務樹：一行一個任務，用縮排（Tab 或空白）表示是上一個縮排較少那行的子任務。
/// 行首的項目符號（-、*、•、1.、1)、[ ]）會拿掉，空白行略過。
pub struct TaskOutline;

impl TaskOutline {
    pub fn parse(text: Option<&str>) -> Vec<OutlineNode> {
        let text = text.unwrap_or("").replace("\r\n", "\n");
        // 先記下每個節點的路徑（從根開始的索引），最後再組成樹；stack 是 (縮排, 路徑)
        let mut roots: Vec<OutlineNode> = Vec::new();
        let mut stack: Vec<(usize, Vec<usize>)> = Vec::new();
        for raw in text.split('\n') {
            let mut indent = 0usize;
            for c in raw.chars() {
                match c {
                    ' ' => indent += 1,
                    '\t' => indent += 4,
                    '　' => indent += 2,
                    _ => break,
                }
            }
            let title = strip_bullet(raw.trim());
            if title.is_empty() {
                continue;
            }
            let title: String = if title.chars().count() > 200 { title.chars().take(200).collect() } else { title };
            let node = OutlineNode::new(title, Vec::new());
            while stack.last().is_some_and(|(i, _)| *i >= indent) {
                stack.pop();
            }
            let path = match stack.last() {
                None => {
                    roots.push(node);
                    vec![roots.len() - 1]
                }
                Some((_, parent_path)) => {
                    let parent = node_at(&mut roots, parent_path);
                    parent.children.push(node);
                    let mut path = parent_path.clone();
                    path.push(parent.children.len() - 1);
                    path
                }
            };
            stack.push((indent, path));
        }
        roots
    }

    pub fn count(nodes: &[OutlineNode]) -> usize {
        nodes.iter().map(OutlineNode::count).sum()
    }
}

fn node_at<'a>(roots: &'a mut [OutlineNode], path: &[usize]) -> &'a mut OutlineNode {
    let mut node = &mut roots[path[0]];
    for &i in &path[1..] {
        node = &mut node.children[i];
    }
    node
}

fn strip_bullet(line: &str) -> String {
    if line.starts_with("[ ]") || line.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("[x]")) {
        return line[3..].trim().to_owned();
    }
    let chars: Vec<char> = line.chars().collect();
    if chars.len() > 1 && matches!(chars[0], '-' | '*' | '•' | '・' | '+') && chars[1] == ' ' {
        return chars[2..].iter().collect::<String>().trim().to_owned();
    }
    let mut i = 0;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    // 1. 標題、1) 標題、1、標題
    let numbered =
        i > 0 && i + 1 < chars.len() && (chars[i] == '、' || matches!(chars[i], '.' | ')') && chars[i + 1] == ' ');
    if numbered {
        return chars[i + 1..].iter().collect::<String>().trim().to_owned();
    }
    line.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 對應 NewToolsTests.TaskOutline_*

    fn titles(nodes: &[OutlineNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.title.as_str()).collect()
    }

    #[test]
    fn task_outline_縮排是子任務_項目符號與編號拿掉() {
        let nodes = TaskOutline::parse(Some(
            "- 第一階段\n  1. 規劃\n  2. 開發\n      [ ] 後端\n  \t前端\n• 第二階段\n\n3、驗收",
        ));
        assert_eq!(vec!["第一階段", "第二階段", "驗收"], titles(&nodes));
        assert_eq!(vec!["規劃", "開發"], titles(&nodes[0].children));
        assert_eq!(vec!["後端", "前端"], titles(&nodes[0].children[1].children));
        assert_eq!(7, TaskOutline::count(&nodes));
    }

    #[test]
    fn task_outline_一般文字不誤刪_空白清單是空的() {
        let nodes = TaskOutline::parse(Some("2026 年度規劃\n-負號開頭\n"));
        assert_eq!(vec!["2026 年度規劃", "-負號開頭"], titles(&nodes));
        assert!(TaskOutline::parse(Some("  \n\n")).is_empty());
    }
}
