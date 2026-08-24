//! `xml_dom` —— 极小 XML DOM（quick-xml 事件流 → 树），docx 结构模型的解析底座。
//!
//! 为什么自造而不用 roxmltree：quick-xml 已在依赖树（plist 传递，版本锁定 0.41），
//! 「解析一次、递归遍历」的需求百行 DOM 即可满足。OOXML 名字空间前缀按**字面**
//! 匹配（`w:` 前缀是事实标准，与 docx.rs 旧扫描器同一假设——全量 ns 解析属过度工程）。
//!
//! 设计取舍：
//! - 文本节点保留**原始形态**（实体不解码）——解码时机留给消费者（docx_model 用
//!   `decode_entities_into`，与旧扫描器逐字节一致：未知实体原样保留 `&`）。
//! - 自闭合元素展开为无子元素的 Element（配置 `expand_empty_elements`）。
//! - 注释 / PI / DOCTYPE 丢弃；CDATA 视作已解码文本。
//! - 迭代建树（无递归），后续递归遍历只面对正常 OOXML 深度（非对抗输入）。

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::error::{AppError, AppResult};

/// 一个 XML 元素：名字（含前缀，如 `w:p`）+ 属性 + 子节点。
pub(super) struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

pub(super) enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    /// 取属性值（名字字面匹配，如 `w:val`）；重复属性取首个。
    pub(super) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// 直接子元素（跳过文本节点）。
    pub(super) fn child_elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// 元素内全部字符数据拼接（**不含实体解码**，保持原始形态）。
    pub(super) fn raw_text(&self) -> String {
        let mut out = String::new();
        fn collect(el: &Element, out: &mut String) {
            for n in &el.children {
                match n {
                    Node::Text(t) => out.push_str(t),
                    Node::Element(e) => collect(e, out),
                }
            }
        }
        collect(self, &mut out);
        out
    }
}

/// 解析整篇 XML，返回根元素（OOXML 即 `w:document`）。
pub(super) fn parse(xml: &str) -> AppResult<Element> {
    let mut reader = Reader::from_str(xml);
    {
        let cfg = reader.config_mut();
        cfg.trim_text(false); // 保留原始空白（w:t xml:space="preserve" 语义）
        cfg.expand_empty_elements = true; // <w:tab/> → Start+End，建树只需一种路径
    }

    let mut stack: Vec<Element> = Vec::new();
    let mut roots: Vec<Element> = Vec::new();
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| AppError::Internal(format!("XML 解析失败: {e}")))?;
        match ev {
            Event::Start(e) => stack.push(element_from_event(&e, reader.decoder())),
            // expand_empty_elements=true 时理论上不再出现；防御性处理
            Event::Empty(e) => stack.push(element_from_event(&e, reader.decoder())),
            Event::End(_) => {
                let el = stack.pop().ok_or_else(|| {
                    AppError::Internal("XML 结构错误：End 无对应 Start".to_string())
                })?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Element(el)),
                    None => roots.push(el),
                }
            }
            Event::Text(t) => {
                if let Some(parent) = stack.last_mut() {
                    let raw = String::from_utf8_lossy(&t).into_owned();
                    parent.children.push(Node::Text(raw));
                }
            }
            // 实体引用独立成事件（0.41 行为）：内容是 `&` 与 `;` 之间的部分，
            // 原样拼回 `&…;` 保住 raw 形态——解码时机留给消费者
            Event::GeneralRef(g) => {
                if let Some(parent) = stack.last_mut() {
                    let mut raw = String::from("&");
                    raw.push_str(&String::from_utf8_lossy(&g));
                    raw.push(';');
                    parent.children.push(Node::Text(raw));
                }
            }
            Event::CData(t) => {
                // CDATA 内容无实体语义，视作已解码文本
                if let Some(parent) = stack.last_mut() {
                    let raw = String::from_utf8_lossy(&t).into_owned();
                    parent.children.push(Node::Text(raw));
                }
            }
            Event::Eof => break,
            _ => {} // Comment / PI / Decl / DocType 丢弃
        }
    }
    // 尾部未闭合（malformed）→ 报错而非吞
    if let Some(unclosed) = stack.pop() {
        return Err(AppError::Internal(format!(
            "XML 结构错误：元素 <{}> 未闭合",
            unclosed.name
        )));
    }
    // 正常 XML 单根；多根（测试用的裸块序列）包一层合成根，统一成树
    match roots.len() {
        0 => Err(AppError::Internal("XML 无根元素".to_string())),
        1 => Ok(roots.pop().expect("len==1 已判")),
        _ => Ok(Element {
            name: "#roots".to_string(),
            attrs: Vec::new(),
            children: roots.into_iter().map(Node::Element).collect(),
        }),
    }
}

/// 从 Start/Empty 事件构造 Element（属性值做标准 XML 反转义）。
fn element_from_event(e: &quick_xml::events::BytesStart<'_>, decoder: quick_xml::Decoder) -> Element {
    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut attrs = Vec::new();
    for attr in e.attributes().with_checks(false) {
        let Ok(attr) = attr else { continue };
        let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
        // decoded_and_normalized_value 处理标准实体（XML 1.0 归一化）；失败退回原始
        // （OOXML 属性极少含实体）。OOXML 声明 version="1.0" → Explicit1_0
        let value = attr
            .decoded_and_normalized_value(quick_xml::XmlVersion::Explicit1_0, decoder)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).into_owned());
        attrs.push((key, value));
    }
    Element {
        name,
        attrs,
        children: Vec::new(),
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_tree_with_attrs() {
        let el = parse(r#"<w:p w:val="x"><w:r><w:t>你好</w:t></w:r></w:p>"#).unwrap();
        assert_eq!(el.name, "w:p");
        assert_eq!(el.attr("w:val"), Some("x"));
        assert_eq!(el.attr("w:missing"), None);
        let r = el.child_elements().next().unwrap();
        assert_eq!(r.name, "w:r");
        assert_eq!(el.raw_text(), "你好");
    }

    #[test]
    fn self_closing_expanded() {
        let el = parse(r#"<w:r><w:tab/><w:t>A</w:t></w:r>"#).unwrap();
        assert_eq!(el.child_elements().count(), 2);
        assert_eq!(el.raw_text(), "A");
    }

    #[test]
    fn text_keeps_raw_entities() {
        // 原始形态：实体不解码（解码时机在消费者）
        let el = parse(r#"<w:t>a &amp; b</w:t>"#).unwrap();
        assert_eq!(el.raw_text(), "a &amp; b");
    }

    #[test]
    fn whitespace_preserved() {
        let el = parse(r#"<w:t xml:space="preserve"> V1.</w:t>"#).unwrap();
        assert_eq!(el.raw_text(), " V1.");
    }

    #[test]
    fn malformed_unclosed_errors() {
        assert!(parse(r#"<w:p><w:r></w:p>"#).is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn comments_and_pi_dropped() {
        let el = parse(r#"<w:p><!-- 注释 --><?pi data?><w:t>x</w:t></w:p>"#).unwrap();
        assert_eq!(el.raw_text(), "x");
        assert_eq!(el.child_elements().count(), 1);
    }
}
