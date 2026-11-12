//! 對應 C# `Core/ProjectGuess.cs`：替資料夾猜要建在哪個 Hayaku 專案。

use crate::ci_set::eq_ic;

pub struct ProjectGuess;

impl ProjectGuess {
    /// 依序：這個資料夾上次用的專案 → 專案名稱或代碼跟資料夾名稱相同 → 互相包含（至少 3 個字）→ 正在計時的任務所屬專案；
    /// 都沒有回 None。projects 是 (代碼, 名稱)。
    pub fn guess<S: AsRef<str>>(
        projects: &[(String, String)],
        hinted: Option<&str>,
        folder_name: &str,
        running_projects: impl IntoIterator<Item = S>,
    ) -> Option<String> {
        if let Some(hinted) = hinted
            && let Some((key, _)) = projects.iter().find(|(k, _)| eq_ic(k, hinted))
        {
            return Some(key.clone());
        }
        let folder = simplify(folder_name);
        if !folder.is_empty() {
            for (key, name) in projects {
                if simplify(name) == folder || simplify(key) == folder {
                    return Some(key.clone());
                }
            }
            let folder_len = folder.chars().count();
            for (key, name) in projects {
                let name = simplify(name);
                if name.chars().count() >= 3 && folder_len >= 3 && (name.contains(&folder) || folder.contains(&name)) {
                    return Some(key.clone());
                }
            }
        }
        for running in running_projects {
            if let Some((key, _)) = projects.iter().find(|(k, _)| eq_ic(k, running.as_ref())) {
                return Some(key.clone());
            }
        }
        None
    }
}

/// 只留字母與數字（含中文），轉小寫：Print-App、print_app、Print App 都一樣。
fn simplify(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 對應 AutoCreateTests.ProjectGuess_*

    fn projects() -> Vec<(String, String)> {
        [("F2P", "F2P 列印平台"), ("PRT", "Print App"), ("EG", "長榮 EHMS")]
            .into_iter()
            .map(|(k, n)| (k.to_owned(), n.to_owned()))
            .collect()
    }

    #[test]
    fn project_guess_依序猜專案() {
        let cases: [(Option<&str>, &str, &str); 5] = [
            (Some("EG"), "Print-App", "EG"),       // 上次用的專案最優先
            (None, "Print-App", "PRT"),            // 名稱相同（忽略大小寫與符號）
            (None, "f2p", "F2P"),                  // 代碼相同
            (None, "EHMS", "EG"),                  // 互相包含
            (Some("XX"), "unknown-folder", "F2P"), // 上次的專案已不存在 → 正在計時的
        ];
        for (hinted, folder, expected) in cases {
            assert_eq!(
                Some(expected.to_owned()),
                ProjectGuess::guess(&projects(), hinted, folder, ["F2P"]),
                "{folder}"
            );
        }
    }

    #[test]
    fn project_guess_都猜不到回null() {
        assert_eq!(None, ProjectGuess::guess(&projects(), None, "random", Vec::<String>::new()));
    }
}
