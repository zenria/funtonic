use crate::config::ExecutorConfig;
use crate::{PROTOCOL_VERSION, VERSION};
use anyhow::Context;
use get_if_addrs::{IfAddr, Interface};
use grpc_service::grpc_protocol::{GetTasksRequest, PublicKey, ValueList, ValueMap};
use os_info::Info;
use query_parser::MatchResult::Rejected;
use query_parser::{MatchResult, Query, QueryMatcher};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::convert::TryFrom;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Tag {
    Map(HashMap<String, Tag>),
    List(Vec<Tag>),
    Value(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExecutorMeta {
    client_id: String,
    version: String,
    tags: HashMap<String, Tag>,
}

impl From<&ExecutorConfig> for ExecutorMeta {
    fn from(config: &ExecutorConfig) -> Self {
        Self {
            client_id: config.client_id.clone(),
            version: VERSION.into(),
            tags: config.tags.clone(),
        }
    }
}

// os info
impl From<Info> for Tag {
    fn from(info: Info) -> Self {
        let mut tags = HashMap::new();
        tags.insert("type".to_string(), Tag::Value(info.os_type().to_string()));
        tags.insert(
            "version".to_string(),
            Tag::Value(info.version().to_string()),
        );
        Tag::Map(tags)
    }
}

impl From<Vec<Interface>> for Tag {
    fn from(interfaces: Vec<Interface>) -> Self {
        interfaces
            .into_iter()
            .fold(HashMap::new(), |mut interfaces, interface| {
                match &interface.addr {
                    IfAddr::V4(ip) => {
                        let if_type = if ip.ip.is_loopback() || interface.name.starts_with("lo:") {
                            "loopback"
                        } else if ip.ip.is_private() {
                            "lan"
                        } else if ip.ip.is_multicast() {
                            // should not happen
                            "multicast"
                        } else if ip.ip.is_broadcast() {
                            // should not happen
                            "broadcast"
                        } else if ip.ip.is_documentation() {
                            // should not happen
                            "documentation"
                        } else if ip.ip.is_unspecified() {
                            // should not happen
                            "unspecified"
                        } else if ip.ip.is_link_local() {
                            // should not happen
                            "link_local"
                        } else if ip.ip.is_documentation() {
                            // should not happen
                            "documentation"
                        } else {
                            "wan"
                        };

                        let if_list = interfaces.entry(if_type).or_insert(HashMap::new());
                        let if_addrs = if_list.entry(interface.name).or_insert(vec![]);
                        let mut addr = HashMap::new();
                        addr.insert("ip", ip.ip.to_string());
                        addr.insert("netmask", ip.netmask.to_string());
                        if let Some(broadcast) = ip.broadcast.as_ref() {
                            addr.insert("broadcast", broadcast.to_string());
                        }
                        if_addrs.push(addr);
                    }
                    IfAddr::V6(_) => { // ignore ipv6 completely
                    }
                }
                interfaces
            })
            .into()
    }
}

impl TryFrom<&ExecutorConfig> for GetTasksRequest {
    type Error = anyhow::Error;

    fn try_from(config: &ExecutorConfig) -> Result<Self, Self::Error> {
        let mut m: ExecutorMeta = config.into();
        // add os info to executor metas
        m.tags.insert("os_info".into(), os_info::get().into());
        m.tags.insert(
            "network_interfaces".into(),
            get_if_addrs::get_if_addrs()?.into(),
        );
        Ok(Self {
            client_id: m.client_id.clone(),
            client_version: m.version.clone(),
            tags: m
                .tags
                .iter()
                .map(|(tag_name, tag_value)| {
                    (
                        tag_name.clone(),
                        grpc_service::grpc_protocol::Tag::from(tag_value),
                    )
                })
                .collect(),
            client_protocol_version: PROTOCOL_VERSION.into(),
            authorized_keys: config
                .authorized_keys
                .iter()
                .try_fold::<_, _, Result<_, anyhow::Error>>(vec![], |mut keys, (id, key)| {
                    keys.push(PublicKey {
                        key_id: id.clone(),
                        key_bytes: base64::decode(key)
                            .with_context(|| format!("Unable to decode key {}", id))?,
                    });
                    Ok(keys)
                })?,
        })
    }
}

impl From<&GetTasksRequest> for ExecutorMeta {
    fn from(r: &GetTasksRequest) -> Self {
        Self {
            client_id: r.client_id.clone(),
            version: r.client_version.clone(),
            tags: r
                .tags
                .iter()
                .map(|(tag_name, tag_value)| (tag_name.clone(), tag_value.into()))
                .collect(),
        }
    }
}

// protobuf types are really painful
impl From<&Tag> for grpc_service::grpc_protocol::Tag {
    fn from(t: &Tag) -> Self {
        Self {
            tag: Some(match t {
                Tag::Map(m) => grpc_service::grpc_protocol::tag::Tag::ValueMap(ValueMap {
                    values: m.iter().map(|(k, tag)| (k.clone(), tag.into())).collect(),
                }),
                Tag::List(l) => grpc_service::grpc_protocol::tag::Tag::ValueList(ValueList {
                    values: l.iter().map(|v| v.into()).collect(),
                }),
                Tag::Value(v) => grpc_service::grpc_protocol::tag::Tag::Value(v.clone()),
            }),
        }
    }
}
// protobuf types are really painful
impl From<&grpc_service::grpc_protocol::Tag> for Tag {
    fn from(t: &grpc_service::grpc_protocol::Tag) -> Self {
        match t.tag.as_ref().unwrap() {
            grpc_service::grpc_protocol::tag::Tag::Value(v) => Tag::Value(v.clone()),
            grpc_service::grpc_protocol::tag::Tag::ValueMap(m) => Tag::Map(
                m.values
                    .iter()
                    .map(|(k, tag)| (k.clone(), tag.into()))
                    .collect(),
            ),
            grpc_service::grpc_protocol::tag::Tag::ValueList(l) => {
                Tag::List(l.values.iter().map(|v| v.into()).collect())
            }
        }
    }
}

impl From<String> for Tag {
    fn from(v: String) -> Self {
        Tag::Value(v)
    }
}
impl From<&str> for Tag {
    fn from(v: &str) -> Self {
        Tag::Value(v.into())
    }
}

impl<S: Into<String>, T: Into<Tag>> From<HashMap<S, T>> for Tag {
    fn from(map: HashMap<S, T, RandomState>) -> Self {
        Tag::Map(
            map.into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }
}
impl<T: Into<Tag>> From<Vec<T>> for Tag {
    fn from(list: Vec<T>) -> Self {
        Tag::List(list.into_iter().map(|value| value.into()).collect())
    }
}

impl QueryMatcher for Tag {
    fn qmatches(&self, query: &Query) -> MatchResult {
        match self {
            Tag::Map(map) => map.qmatches(query),
            Tag::List(list) => list.qmatches(query),
            Tag::Value(v) => v.qmatches(query),
        }
    }
}

/// Used in ExecutorMeta QueryMatcher impl to avoid cloning large structs
enum TagRef<'a> {
    Map(&'a HashMap<String, Tag>),
    Value(&'a str),
}

impl<'a> QueryMatcher for TagRef<'a> {
    fn qmatches(&self, query: &Query) -> MatchResult {
        match self {
            TagRef::Map(map) => map.qmatches(query),
            TagRef::Value(value) => value.qmatches(query),
        }
    }
}
impl QueryMatcher for ExecutorMeta {
    fn qmatches(&self, query: &Query) -> MatchResult {
        vec![TagRef::Value(&self.client_id), TagRef::Map(&self.tags)].qmatches(query)
    }
}

impl ExecutorMeta {
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn tags(&self) -> &HashMap<String, Tag> {
        &self.tags
    }

    pub fn tags_mut(&mut self) -> &mut HashMap<String, Tag> {
        &mut self.tags
    }
}

#[cfg(test)]
mod test {
    use crate::executor_meta::{ExecutorMeta, Tag};
    use query_parser::{parse, QueryMatcher};
    use std::collections::HashMap;

    trait TestMatch {
        fn matches(&self, query: &str) -> bool;
    }
    impl<T: QueryMatcher> TestMatch for T {
        fn matches(&self, query: &str) -> bool {
            dbg!(&query);
            self.qmatches(&parse(query).unwrap()).matches()
        }
    }

    #[test]
    fn tag_match() {
        let coucou = Tag::Value("coucou".into());
        assert!(coucou.matches("coucou"));
        assert!(!coucou.matches("foo"));
        assert!(coucou.matches("*"));

        let foo_bar = Tag::List(vec!["foo".into(), "bar".into()]);
        assert!(foo_bar.matches("*"));
        assert!(foo_bar.matches("foo"));
        assert!(foo_bar.matches("bar"));
        assert!(!foo_bar.matches("fooo"));

        let mut maap: HashMap<String, Tag> = HashMap::new();
        maap.insert(String::from("key1"), "value1".into());
        maap.insert(String::from("key2"), "value2".into());
        let maap = Tag::Map(maap);
        assert!(maap.matches("*"));
        assert!(maap.matches("key1:value1"));
        assert!(maap.matches("key2:value2"));
        assert!(!maap.matches("value3"));
        assert!(!maap.matches("key1:value2"));
        assert!(maap.matches("key1:*"));
    }

    #[test]
    fn deser_tag() {
        serde_yaml::from_str::<Tag>("bar").unwrap();
        serde_yaml::from_str::<Tag>(r#"["bar", "foo"]"#).unwrap();
        serde_yaml::from_str::<Tag>("- foo\n- bar").unwrap();
        serde_yaml::from_str::<Tag>("key1: value1\nkey2: value2").unwrap();

        serde_yaml::from_str::<HashMap<String, Tag>>("tag1: bar\ntag2: foo").unwrap();
        serde_yaml::from_str::<HashMap<String, Tag>>("tag1:\n  - bar\n  - foo").unwrap();
        serde_yaml::from_str::<HashMap<String, Tag>>("tag1:\n  - bar\n  - foo\ntag2: coucou")
            .unwrap();
        serde_yaml::from_str::<HashMap<String, Tag>>(
            "tag1:\n  - bar\n  - foo\ntag2: coucou\ntag3: bar\ntag_map:\n  foo: bar\n  bar: foo",
        )
        .unwrap();

        serde_yaml::from_str::< HashMap < String, Tag > > ("env: prod\nroles:\n  - foo\n  - bar\nos:\n  type: Linux\n  sub_type: Ubuntu\n  version: \"18.04\"")
.unwrap();
    }

    #[test]
    fn meta_match() {
        let metas = r#"
        client_id: siderant
        version: 0.0.1
        tags:
          env: prod
          roles:
            - foo
            - bar
          os:
            type: Linux
            sub_type: Ubuntu
            version: "18.04"
        "#;
        let meta: ExecutorMeta = serde_yaml::from_str(metas).unwrap();

        assert!(meta.matches("*"));
        assert!(meta.matches("siderant"));
        assert!(!meta.matches("prod"));
        assert!(meta.matches("env:*"));
        assert!(meta.matches("env:prod"));
        assert!(!meta.matches("env:dev"));
        assert!(meta.matches("roles:*"));
        assert!(meta.matches("roles:foo"));
        assert!(meta.matches("roles:bar"));
        assert!(!meta.matches("non_existing:bar"));
        assert!(!meta.matches("non_existing:*"));

        assert!(meta.matches("os:*"));
        assert!(meta.matches("os:type:Linux"));
        assert!(meta.matches("os:type:*"));
        assert!(meta.matches("os:version:18.04"));
        assert!(!meta.matches("os:type:Windows"));

        assert!(meta.matches("env:prod and siderant"));
        assert!(!meta.matches("env:prod and !siderant"));
        // this is a TODO
    }
}
