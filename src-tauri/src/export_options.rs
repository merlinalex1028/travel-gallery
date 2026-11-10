use crate::{
    db,
    error::{required, AppError, Result},
    id,
};
use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamingRule {
    pub pattern: String,
    pub start: u32,
    pub padding: u8,
}
impl NamingRule {
    pub fn validate(&self) -> Result<()> {
        if self.pattern.trim().is_empty()
            || self.pattern.len() > 300
            || !(1..=999_999_999).contains(&self.start)
            || !(1..=9).contains(&self.padding)
        {
            return Err(AppError::new(
                "INVALID_FILTER",
                "命名规则、起始序号或序号位数无效",
            ));
        }
        let mut rest = self.pattern.as_str();
        while let Some(start) = rest.find('{') {
            if rest[..start].contains('}') {
                return Err(AppError::new("INVALID_FILTER", "命名占位符不完整"));
            }
            let Some(end) = rest[start..].find('}').map(|i| start + i) else {
                return Err(AppError::new("INVALID_FILTER", "命名占位符不完整"));
            };
            if !["date", "time", "original", "source", "seq"].contains(&&rest[start + 1..end]) {
                return Err(AppError::new(
                    "INVALID_FILTER",
                    "只支持 date、time、original、source 和 seq 占位符",
                ));
            }
            rest = &rest[end + 1..];
        }
        if rest.contains('}')
            || self
                .pattern
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        {
            return Err(AppError::new(
                "INVALID_FILTER",
                "命名规则不能包含路径或文件名禁用字符",
            ));
        }
        Ok(())
    }
    pub fn filename(
        &self,
        filename: &str,
        source: &str,
        capture: Option<&str>,
        index: usize,
    ) -> Result<String> {
        let path = Path::new(filename);
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let extension = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let date = capture
            .and_then(|s| s.get(..10))
            .map(|s| s.replace('-', ""))
            .unwrap_or_else(|| "未知日期".into());
        let time = capture
            .and_then(|s| s.get(11..19))
            .map(|s| s.replace(':', ""))
            .unwrap_or_else(|| "未知时间".into());
        let sequence = (self.start as u64)
            .checked_add(index as u64)
            .ok_or_else(|| AppError::new("INVALID_FILTER", "序号超出范围"))?;
        // Substitute only template tokens, never tokens contained in original names.
        let mut output = String::new();
        let mut rest = self.pattern.as_str();
        while let Some(start) = rest.find('{') {
            output.push_str(&rest[..start]);
            let end = rest[start..]
                .find('}')
                .map(|i| i + start)
                .ok_or_else(|| AppError::new("INVALID_FILTER", "命名占位符不完整"))?;
            output.push_str(&match &rest[start + 1..end] {
                "date" => date.clone(),
                "time" => time.clone(),
                "original" => stem.to_string(),
                "source" => source.to_string(),
                "seq" => format!("{sequence:0width$}", width = self.padding as usize),
                _ => return Err(AppError::new("INVALID_FILTER", "命名占位符无效")),
            });
            rest = &rest[end + 1..];
        }
        output.push_str(rest);
        output.push_str(&extension);
        validate_filename(&output)?;
        Ok(output)
    }
}
pub fn validate_filename(name: &str) -> Result<()> {
    let base = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'));
    if name.is_empty()
        || name.len() > 240
        || name.ends_with(['.', ' '])
        || reserved
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || [".", ".."].contains(&name)
    {
        return Err(AppError::new(
            "INVALID_FILTER",
            "生成的文件名不适用于 Windows/macOS，或超过 240 字节",
        ));
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportOptions {
    pub layout: String,
    pub conflict_policy: String,
    pub include_linked: bool,
    pub include_xmp: bool,
    #[serde(default)]
    pub naming: Option<NamingRule>,
}
impl ExportOptions {
    pub fn validate(&self) -> Result<()> {
        if !["flat", "captureMonth", "sourceRelative"].contains(&self.layout.as_str())
            || !["skip", "rename"].contains(&self.conflict_policy.as_str())
        {
            return Err(AppError::new(
                "INVALID_FILTER",
                "导出目录或冲突规则无效，不支持覆盖",
            ));
        }
        if let Some(rule) = &self.naming {
            rule.validate()?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub options: ExportOptions,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Presets {
    version: u8,
    presets: Vec<Preset>,
}
const KEY: &str = "exportPresetsV1";
pub fn list(c: &Connection) -> Result<Vec<Preset>> {
    let Some(value) = db::setting(c, KEY)? else {
        return Ok(vec![]);
    };
    let data: Presets = serde_json::from_str(&value)?;
    if data.version != 1 || data.presets.len() > 100 {
        return Err(AppError::new("INVALID_FILTER", "导出预设版本或数量无效"));
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for preset in &data.presets {
        valid_name(&preset.name)?;
        preset.options.validate()?;
        if preset.revision == 0
            || preset.id.is_empty()
            || !ids.insert(&preset.id)
            || !names.insert(preset.name.to_lowercase())
        {
            return Err(AppError::new("INVALID_FILTER", "导出预设记录无效或重名"));
        }
    }
    Ok(data.presets)
}
fn valid_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(AppError::new(
            "INVALID_FILTER",
            "预设名称须为 1–80 个可见字符",
        ));
    }
    Ok(())
}
pub fn mutate(c: &mut Connection, command: &str, r: &Value) -> Result<Value> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut presets = list(&tx)?;
    if r.get("id").is_some_and(|value| !value.is_string()) {
        return Err(AppError::new("INVALID_FILTER", "预设标识无效"));
    }
    let key = r["id"].as_str();
    let existing = key.and_then(|id| presets.iter().position(|p| p.id == id));
    if key.is_some() && existing.is_none() {
        return Err(AppError::new("DB_CONFLICT", "预设已移除，请刷新后重试"));
    }
    if let Some(index) = existing {
        if r["expectedRevision"].as_u64() != Some(presets[index].revision) {
            return Err(AppError::new("DB_CONFLICT", "预设已改变，请重新选择后编辑"));
        }
    }
    let result = if command == "delete_export_preset" {
        let index =
            existing.ok_or_else(|| AppError::new("INVALID_FILTER", "请选择要移除的预设"))?;
        presets.remove(index);
        json!({"updated":true})
    } else {
        let name = required(r, "name")?.trim();
        valid_name(name)?;
        let options: ExportOptions = serde_json::from_value(r["options"].clone())?;
        options.validate()?;
        if presets
            .iter()
            .any(|p| Some(p.id.as_str()) != key && p.name.to_lowercase() == name.to_lowercase())
        {
            return Err(AppError::new("DB_CONFLICT", "已有同名导出预设"));
        }
        let preset = Preset {
            id: key.map(str::to_owned).unwrap_or_else(id),
            name: name.into(),
            revision: existing
                .map(|i| {
                    presets[i]
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| AppError::new("DB_CONFLICT", "预设版本超出范围"))
                })
                .transpose()?
                .unwrap_or(1),
            options,
        };
        if let Some(index) = existing {
            presets[index] = preset.clone();
        } else {
            if presets.len() >= 100 {
                return Err(AppError::new("INVALID_FILTER", "最多保存 100 个导出预设"));
            }
            presets.push(preset.clone());
        }
        json!(preset)
    };
    tx.execute(
        "INSERT OR REPLACE INTO app_settings(key,value_json) VALUES(?,?)",
        rusqlite::params![
            KEY,
            serde_json::to_string(&Presets {
                version: 1,
                presets
            })?
        ],
    )?;
    tx.commit()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_preserve_extensions_dates_unicode_and_literal_tokens() {
        let rule = NamingRule {
            pattern: "{date}_{time}_{seq}_{original}".into(),
            start: 7,
            padding: 4,
        };
        rule.validate().unwrap();
        assert_eq!(
            rule.filename("山野{seq}.JPG", "来源", Some("2026-06-15 09:20:00"), 1)
                .unwrap(),
            "20260615_092000_0008_山野{seq}.JPG"
        );
        assert_eq!(
            rule.filename("A.CR2", "来源", None, 0).unwrap(),
            "未知日期_未知时间_0007_A.CR2"
        );
        for pattern in ["../{seq}", "{bad}", "{seq", "}x", "a:b", ""] {
            assert!(
                NamingRule {
                    pattern: pattern.into(),
                    ..rule.clone()
                }
                .validate()
                .is_err(),
                "{pattern}"
            );
        }
        for name in [
            "CON.jpg",
            "lpt9.JPG",
            "nul.any.jpg",
            "bad. ",
            "A:B.jpg",
            "../x.jpg",
        ] {
            assert!(validate_filename(name).is_err(), "{name}");
        }
        assert!(validate_filename(&format!("{}.jpg", "山".repeat(80))).is_err());
    }
}
