use std::collections::BTreeMap;
use std::ops::Index;
use std::string;
use std::i64;
use std::f64;
use std::mem;
use std::vec;
use parser::*;
use scanner::{TScalarStyle, ScanError, TokenType, Marker};
use linked_hash_map::LinkedHashMap;
use std::ops::Deref;

use std::cmp::Ordering;
use std::hash::Hasher;

/// A YAML node is stored as this `Node` enumeration, which provides an easy way to
/// access your YAML document.
///
/// # Examples
///
/// ```
/// use yaml_rust::{Yaml, Node};
/// let foo = Node::from_str("-123"); // convert the string to the appropriate YAML type
/// assert_eq!(foo.as_i64().unwrap(), -123);
///
/// // iterate over an Array
/// let vec = Node::Array(vec![Yaml(None, Node::Integer(1)), Yaml(None, Node::Integer(2))]);
/// for v in vec.as_vec().unwrap() {
///     assert!(v.as_i64().is_some());
/// }
/// ```
#[derive(Clone, PartialEq, PartialOrd, Debug, Eq, Ord, Hash)]
pub enum Node {
    /// Float types are stored as String and parsed on demand.
    /// Note that f64 does NOT implement Eq trait and can NOT be stored in BTreeMap.
    Real(string::String),
    /// YAML int is stored as i64.
    Integer(i64),
    /// YAML scalar.
    String(string::String),
    /// YAML bool, e.g. `true` or `false`.
    Boolean(bool),
    /// YAML array, can be accessed as a `Vec`.
    Array(self::Array),
    /// YAML hash, can be accessed as a `LinkedHashMap`.
    ///
    /// Itertion order will match the order of insertion into the map.
    Hash(self::Hash),
    /// Alias, not fully supported yet.
    Alias(usize),
    /// YAML null, e.g. `null` or `~`.
    Null,
    /// Accessing a nonexistent node via the Index trait returns `BadValue`. This
    /// simplifies error handling in the calling code. Invalid type conversion also
    /// returns `BadValue`.
    BadValue,
}

#[derive(Clone, Debug)]
pub struct Yaml(pub Option<Marker>, pub Node);

impl Deref for Yaml {
    type Target = Node;

    fn deref(&self) -> &Node {
        &self.1
    }
}

impl PartialEq for Yaml {
    fn eq(&self, other: &Yaml) -> bool {
        self.1 == other.1
    }
}

impl PartialOrd for Yaml {
    fn partial_cmp(&self, other: &Yaml) -> Option<Ordering> {
        Some(self.1.cmp(&other.1))
    }
}

impl Eq for Yaml {}

impl Ord for Yaml {
    fn cmp(&self, other: &Yaml) -> Ordering {
        self.1.cmp(&other.1)
    }
}

impl ::std::hash::Hash for Yaml {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.1.hash(state);
    }
}


pub type Array = Vec<Yaml>;

#[derive(Clone, Debug)]
pub struct HashItem { pub key_marker: Option<Marker>, pub value: Yaml }
pub type Hash = LinkedHashMap<Node, HashItem>; // don't store mark in key; we want to be able to look up by value.

impl PartialEq for HashItem {
    fn eq(&self, other: &HashItem) -> bool {
        self.value == other.value
    }
}

impl PartialOrd for HashItem {
    fn partial_cmp(&self, other: &HashItem) -> Option<Ordering> {
        Some(self.value.cmp(&other.value))
    }
}

impl Eq for HashItem {}

impl Ord for HashItem {
    fn cmp(&self, other: &HashItem) -> Ordering {
        self.value.cmp(&other.value)
    }
}

impl ::std::hash::Hash for HashItem {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}


// parse f64 as Core schema
// See: https://github.com/chyh1990/yaml-rust/issues/51
fn parse_f64(v: &str) -> Option<f64> {
    match v {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => Some(f64::NEG_INFINITY),
        ".nan" | "NaN" | ".NAN" => Some(f64::NAN),
        _ => v.parse::<f64>().ok()
    }
}

pub struct YamlLoader {
    docs: Vec<Yaml>,
    // states
    // (current node, anchor_id) tuple
    doc_stack: Vec<(Yaml, usize)>,
    key_stack: Vec<Yaml>,
    anchor_map: BTreeMap<usize, Yaml>,
}

impl MarkedEventReceiver for YamlLoader {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        // println!("EV {:?}", ev);
        match ev {
            Event::DocumentStart => {
                // do nothing
            },
            Event::DocumentEnd => {
                match self.doc_stack.len() {
                    // empty document
                    0 => self.docs.push(Yaml(None, Node::BadValue)),
                    1 => self.docs.push(self.doc_stack.pop().unwrap().0),
                    _ => unreachable!()
                }
            },
            Event::SequenceStart(aid) => {
                self.doc_stack.push((Yaml(Some(mark), Node::Array(Vec::new())), aid));
            },
            Event::SequenceEnd => {
                let node = self.doc_stack.pop().unwrap();
                self.insert_new_node(node);
            },
            Event::MappingStart(aid) => {
                self.doc_stack.push((Yaml(Some(mark), Node::Hash(Hash::new())), aid));
                self.key_stack.push(Yaml(Some(mark), Node::BadValue));
            },
            Event::MappingEnd => {
                self.key_stack.pop().unwrap();
                let node = self.doc_stack.pop().unwrap();
                self.insert_new_node(node);
            },
            Event::Scalar(v, style, aid, tag) => {
                let node = if style != TScalarStyle::Plain {
                    Node::String(v)
                } else if let Some(TokenType::Tag(ref handle, ref suffix)) = tag {
                    // XXX tag:yaml.org,2002:
                    if handle == "!!" {
                        match suffix.as_ref() {
                            "bool" => {
                                // "true" or "false"
                                match v.parse::<bool>() {
                                    Err(_) => Node::BadValue,
                                    Ok(v) => Node::Boolean(v)
                                }
                            },
                            "int" => {
                                match v.parse::<i64>() {
                                    Err(_) => Node::BadValue,
                                    Ok(v) => Node::Integer(v)
                                }
                            },
                            "float" => {
                                match parse_f64(&v) {
                                    Some(_) => Node::Real(v),
                                    None => Node::BadValue,
                                }
                            },
                            "null" => {
                                match v.as_ref() {
                                    "~" | "null" => Node::Null,
                                    _ => Node::BadValue,
                                }
                            }
                            _  => Node::String(v),
                        }
                    } else {
                        Node::String(v)
                    }
                } else {
                    // Datatype is not specified, or unrecognized
                    Node::from_str(&v)
                };

                let yaml = Yaml(Some(mark), node);
                self.insert_new_node((yaml, aid));
            },
            Event::Alias(id) => {
                let yaml = match self.anchor_map.get(&id) {
                    Some(v) => v.clone(),
                    None => Yaml(Some(mark), Node::BadValue),
                };
                self.insert_new_node((yaml, 0));
            }
            _ => { /* ignore */ }
        }
        // println!("DOC {:?}", self.doc_stack);
    }
}

impl YamlLoader {
    fn insert_new_node(&mut self, node: (Yaml, usize)) {
        // valid anchor id starts from 1
        if node.1 > 0 {
            self.anchor_map.insert(node.1, node.0.clone());
        }
        if self.doc_stack.is_empty() {
            self.doc_stack.push(node);
        } else {
            let parent = self.doc_stack.last_mut().unwrap();
            match *parent {
                (Yaml(_, Node::Array(ref mut v)), _) => v.push(node.0),
                (Yaml(_, Node::Hash(ref mut h)), _) => {
                    let cur_key = self.key_stack.last_mut().unwrap();
                    // current node is a key
                    if cur_key.is_badvalue() {
                        *cur_key = node.0;
                    // current node is a value
                    } else {
                        let mut newkey = Yaml(None, Node::BadValue);
                        mem::swap(&mut newkey, cur_key);
                        h.insert(newkey.1, HashItem { key_marker: None, value:  node.0});
                    }
                },
                _ => unreachable!(),
            }
        }
    }

    pub fn load_from_str(source: &str) -> Result<Vec<Yaml>, ScanError>{
        let mut loader = YamlLoader {
            docs: Vec::new(),
            doc_stack: Vec::new(),
            key_stack: Vec::new(),
            anchor_map: BTreeMap::new(),
        };
        let mut parser = Parser::new(source.chars());
        try!(parser.load(&mut loader, true));
        Ok(loader.docs)
    }
}

macro_rules! define_as (
    ($name:ident, $t:ident, $yt:ident) => (
pub fn $name(&self) -> Option<$t> {
    match *self {
        Node::$yt(v) => Some(v),
        _ => None
    }
}
    );
);

macro_rules! define_as_ref (
    ($name:ident, $t:ty, $yt:ident) => (
pub fn $name(&self) -> Option<$t> {
    match *self {
        Node::$yt(ref v) => Some(v),
        _ => None
    }
}
    );
);

macro_rules! define_into (
    ($name:ident, $t:ty, $yt:ident) => (
pub fn $name(self) -> Option<$t> {
    match self {
        Node::$yt(v) => Some(v),
        _ => None
    }
}
    );
);

impl Node {
    define_as!(as_bool, bool, Boolean);
    define_as!(as_i64, i64, Integer);

    define_as_ref!(as_str, &str, String);
    define_as_ref!(as_hash, &Hash, Hash);
    define_as_ref!(as_vec, &Array, Array);

    define_into!(into_bool, bool, Boolean);
    define_into!(into_i64, i64, Integer);
    define_into!(into_string, String, String);
    define_into!(into_hash, Hash, Hash);
    define_into!(into_vec, Array, Array);

    pub fn is_null(&self) -> bool {
        match *self {
            Node::Null => true,
            _ => false
        }
    }

    pub fn is_badvalue(&self) -> bool {
        match *self {
            Node::BadValue => true,
            _ => false
        }
    }

    pub fn is_array(&self) -> bool {
        match *self {
            Node::Array(_) => true,
            _ => false
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Node::Real(ref v) => parse_f64(v),
            _ => None
        }
    }

    pub fn into_f64(self) -> Option<f64> {
        match self {
            Node::Real(ref v) => parse_f64(v),
            _ => None
        }
    }
}

#[cfg_attr(feature = "cargo-clippy", allow(should_implement_trait))]
impl Node {
    // Not implementing FromStr because there is no possibility of Error.
    // This function falls back to Node::String if nothing else matches.
    pub fn from_str(v: &str) -> Node {
        if v.starts_with("0x") {
            let n = i64::from_str_radix(&v[2..], 16);
            if n.is_ok() {
                return Node::Integer(n.unwrap());
            }
        }
        if v.starts_with("0o") {
            let n = i64::from_str_radix(&v[2..], 8);
            if n.is_ok() {
                return Node::Integer(n.unwrap());
            }
        }
        if v.starts_with('+') && v[1..].parse::<i64>().is_ok() {
            return Node::Integer(v[1..].parse::<i64>().unwrap());
        }
        match v {
            "~" | "null" => Node::Null,
            "true" => Node::Boolean(true),
            "false" => Node::Boolean(false),
            _ if v.parse::<i64>().is_ok() => Node::Integer(v.parse::<i64>().unwrap()),
            // try parsing as f64
            _ if parse_f64(v).is_some() => Node::Real(v.to_owned()),
            _ => Node::String(v.to_owned())
        }
    }
}

static BAD_VALUE: Yaml= Yaml(None, Node::BadValue);
impl<'a> Index<&'a str> for Node {
    type Output = Yaml;

    fn index(&self, idx: &'a str) -> &Yaml {
        let key = Node::String(idx.to_owned());
        match self.as_hash() {
            Some(h) => h.get(&key).map(|HashItem { value, ..}| value).unwrap_or(&BAD_VALUE),
            None => &BAD_VALUE
        }
    }
}

impl Index<usize> for Node {
    type Output = Yaml;

    fn index(&self, idx: usize) -> &Yaml {
        if let Some(v) = self.as_vec() {
            v.get(idx).unwrap_or(&BAD_VALUE)
        } else if let Some(v) = self.as_hash() {
            let key = Node::Integer(idx as i64);
            v.get(&key).map(|HashItem { value, ..}| value).unwrap_or(&BAD_VALUE)
        } else {
            &BAD_VALUE
        }
    }
}

impl IntoIterator for Node {
    type Item = Yaml;
    type IntoIter = YamlIter;

    fn into_iter(self) -> Self::IntoIter {
        YamlIter {
            yaml: self.into_vec()
                .unwrap_or_else(Vec::new).into_iter()
        }
    }
}

pub struct YamlIter {
    yaml: vec::IntoIter<Yaml>,
}

impl Iterator for YamlIter {
    type Item = Yaml;

    fn next(&mut self) -> Option<Yaml> {
        self.yaml.next()
    }
}

#[cfg(test)]
mod test {
    use yaml::*;
    use std::f64;
    #[test]
    fn test_coerce() {
        let s = "---
a: 1
b: 2.2
c: [1, 2]
";
        let out = YamlLoader::load_from_str(&s).unwrap();
        let doc = &out[0];
        assert_eq!(doc["a"].as_i64().unwrap(), 1i64);
        assert_eq!(doc["b"].as_f64().unwrap(), 2.2f64);
        assert_eq!(doc["c"][1].as_i64().unwrap(), 2i64);
        assert!(doc["d"][0].is_badvalue());
    }

    #[test]
    fn test_empty_doc() {
        let s: String = "".to_owned();
        YamlLoader::load_from_str(&s).unwrap();
        let s: String = "---".to_owned();
        assert_eq!(YamlLoader::load_from_str(&s).unwrap()[0].1, Node::Null);
    }

    #[test]
    fn test_parser() {
        let s: String = "
# comment
a0 bb: val
a1:
    b1: 4
    b2: d
a2: 4 # i'm comment
a3: [1, 2, 3]
a4:
    - - a1
      - a2
    - 2
a5: 'single_quoted'
a6: \"double_quoted\"
a7: 你好
".to_owned();
        let out = YamlLoader::load_from_str(&s).unwrap();
        let doc = &out[0];
        assert_eq!(doc["a7"].as_str().unwrap(), "你好");
    }

    #[test]
    fn test_multi_doc() {
        let s =
"
'a scalar'
---
'a scalar'
---
'a scalar'
";
        let out = YamlLoader::load_from_str(&s).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn test_anchor() {
        let s =
"
a1: &DEFAULT
    b1: 4
    b2: d
a2: *DEFAULT
";
        let out = YamlLoader::load_from_str(&s).unwrap();
        let doc = &out[0];
        assert_eq!(doc["a2"]["b1"].as_i64().unwrap(), 4);
    }

    #[test]
    fn test_bad_anchor() {
        let s =
"
a1: &DEFAULT
    b1: 4
    b2: *DEFAULT
";
        let out = YamlLoader::load_from_str(&s).unwrap();
        let doc = &out[0];
        assert_eq!(doc["a1"]["b2"].1, Node::BadValue);

    }

    #[test]
    fn test_github_27() {
        // https://github.com/chyh1990/yaml-rust/issues/27
        let s = "&a";
        let out = YamlLoader::load_from_str(&s).unwrap();
        let doc = &out[0];
        assert_eq!(doc.as_str().unwrap(), "");
    }

    #[test]
    fn test_plain_datatype() {
        let s =
"
- 'string'
- \"string\"
- string
- 123
- -321
- 1.23
- -1e4
- ~
- null
- true
- false
- !!str 0
- !!int 100
- !!float 2
- !!null ~
- !!bool true
- !!bool false
- 0xFF
# bad values
- !!int string
- !!float string
- !!bool null
- !!null val
- 0o77
- [ 0xF, 0xF ]
- +12345
- [ true, false ]
";
        let out = YamlLoader::load_from_str(&s).unwrap();
        let doc = &out[0];

        assert_eq!(doc[0].as_str().unwrap(), "string");
        assert_eq!(doc[1].as_str().unwrap(), "string");
        assert_eq!(doc[2].as_str().unwrap(), "string");
        assert_eq!(doc[3].as_i64().unwrap(), 123);
        assert_eq!(doc[4].as_i64().unwrap(), -321);
        assert_eq!(doc[5].as_f64().unwrap(), 1.23);
        assert_eq!(doc[6].as_f64().unwrap(), -1e4);
        assert!(doc[7].is_null());
        assert!(doc[8].is_null());
        assert_eq!(doc[9].as_bool().unwrap(), true);
        assert_eq!(doc[10].as_bool().unwrap(), false);
        assert_eq!(doc[11].as_str().unwrap(), "0");
        assert_eq!(doc[12].as_i64().unwrap(), 100);
        assert_eq!(doc[13].as_f64().unwrap(), 2.0);
        assert!(doc[14].is_null());
        assert_eq!(doc[15].as_bool().unwrap(), true);
        assert_eq!(doc[16].as_bool().unwrap(), false);
        assert_eq!(doc[17].as_i64().unwrap(), 255);
        assert!(doc[18].is_badvalue());
        assert!(doc[19].is_badvalue());
        assert!(doc[20].is_badvalue());
        assert!(doc[21].is_badvalue());
        assert_eq!(doc[22].as_i64().unwrap(), 63);
        assert_eq!(doc[23][0].as_i64().unwrap(), 15);
        assert_eq!(doc[23][1].as_i64().unwrap(), 15);
        assert_eq!(doc[24].as_i64().unwrap(), 12345);
        assert!(doc[25][0].as_bool().unwrap());
        assert!(!doc[25][1].as_bool().unwrap());
    }

    #[test]
    fn test_bad_hypen() {
        // See: https://github.com/chyh1990/yaml-rust/issues/23
        let s = "{-";
        assert!(YamlLoader::load_from_str(&s).is_err());
    }

    #[test]
    fn test_issue_65() {
        // See: https://github.com/chyh1990/yaml-rust/issues/65
        let b = "\n\"ll\\\"ll\\\r\n\"ll\\\"ll\\\r\r\r\rU\r\r\rU";
        assert!(YamlLoader::load_from_str(&b).is_err());
    }

    #[test]
    fn test_bad_docstart() {
        assert!(YamlLoader::load_from_str("---This used to cause an infinite loop").is_ok());
        assert_eq!(YamlLoader::load_from_str("----"), Ok(vec![Yaml(None, Node::String(String::from("----")))]));
        assert_eq!(YamlLoader::load_from_str("--- #here goes a comment"), Ok(vec![Yaml(None, Node::Null)]));
        assert_eq!(YamlLoader::load_from_str("---- #here goes a comment"), Ok(vec![Yaml(None, Node::String(String::from("----")))]));
    }

    #[test]
    fn test_plain_datatype_with_into_methods() {
        let s =
"
- 'string'
- \"string\"
- string
- 123
- -321
- 1.23
- -1e4
- true
- false
- !!str 0
- !!int 100
- !!float 2
- !!bool true
- !!bool false
- 0xFF
- 0o77
- +12345
- -.INF
- .NAN
- !!float .INF
";

        let out = YamlLoader::load_from_str(&s).unwrap();
        let first = out.into_iter().next().unwrap();
        let mut doc = first.1.into_iter();

        assert_eq!(doc.next().unwrap().1.into_string().unwrap(), "string");
        assert_eq!(doc.next().unwrap().1.into_string().unwrap(), "string");
        assert_eq!(doc.next().unwrap().1.into_string().unwrap(), "string");
        assert_eq!(doc.next().unwrap().1.into_i64().unwrap(), 123);
        assert_eq!(doc.next().unwrap().1.into_i64().unwrap(), -321);
        assert_eq!(doc.next().unwrap().1.into_f64().unwrap(), 1.23);
        assert_eq!(doc.next().unwrap().1.into_f64().unwrap(), -1e4);
        assert_eq!(doc.next().unwrap().1.into_bool().unwrap(), true);
        assert_eq!(doc.next().unwrap().1.into_bool().unwrap(), false);
        assert_eq!(doc.next().unwrap().1.into_string().unwrap(), "0");
        assert_eq!(doc.next().unwrap().1.into_i64().unwrap(), 100);
        assert_eq!(doc.next().unwrap().1.into_f64().unwrap(), 2.0);
        assert_eq!(doc.next().unwrap().1.into_bool().unwrap(), true);
        assert_eq!(doc.next().unwrap().1.into_bool().unwrap(), false);
        assert_eq!(doc.next().unwrap().1.into_i64().unwrap(), 255);
        assert_eq!(doc.next().unwrap().1.into_i64().unwrap(), 63);
        assert_eq!(doc.next().unwrap().1.into_i64().unwrap(), 12345);
        assert_eq!(doc.next().unwrap().1.into_f64().unwrap(), f64::NEG_INFINITY);
        assert!(doc.next().unwrap().1.into_f64().is_some());
        assert_eq!(doc.next().unwrap().1.into_f64().unwrap(), f64::INFINITY);
    }

    #[test]
    fn test_hash_order() {
        let s = "---
b: ~
a: ~
c: ~
";
        let out = YamlLoader::load_from_str(&s).unwrap();
        let first = out.into_iter().next().unwrap();
        let mut iter = first.1.into_hash().unwrap().into_iter();
        assert_eq!(Some((Node::String("b".to_owned()), HashItem { key_marker: Some(Marker { index: 0, col:3, line:0 }), value: Yaml(None, Node::Null)})), iter.next());
        assert_eq!(Some((Node::String("a".to_owned()), HashItem { key_marker: Some(Marker { index: 0, col:0, line:2 }), value: Yaml(None, Node::Null)})), iter.next());
        assert_eq!(Some((Node::String("c".to_owned()), HashItem { key_marker: Some(Marker { index: 0, col:15, line:0 }), value: Yaml(None, Node::Null)})), iter.next());
        assert_eq!(None, iter.next());
    }

    #[test]
    fn test_integer_key() {
        let s = "
0:
    important: true
1:
    important: false
";
        let out = YamlLoader::load_from_str(&s).unwrap();
        let first = out.into_iter().next().unwrap();
        assert_eq!(first[0]["important"].as_bool().unwrap(), true);
    }
}
