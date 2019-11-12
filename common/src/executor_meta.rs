use crate::config::ExecutorConfig;
use crate::VERSION;
use grpc_service::{GetTasksRequest, ValueList, ValueMap};
use query_parser::{Query, QueryMatcher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

impl From<&ExecutorMeta> for GetTasksRequest {
    fn from(m: &ExecutorMeta) -> Self {
        Self {
            client_id: m.client_id.clone(),
            client_version: m.version.clone(),
            tags: m
                .tags
                .iter()
                .map(|(tag_name, tag_value)| (tag_name.clone(), grpc_service::Tag::from(tag_value)))
                .collect(),
        }
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
impl From<&Tag> for grpc_service::Tag {
    fn from(t: &Tag) -> Self {
        Self {
            tag: Some(match t {
                Tag::Map(m) => grpc_service::tag::Tag::ValueMap(ValueMap {
                    values: m.iter().map(|(k, tag)| (k.clone(), tag.into())).collect(),
                }),
                Tag::List(l) => grpc_service::tag::Tag::ValueList(ValueList {
                    values: l.iter().map(|v| v.into()).collect(),
                }),
                Tag::Value(v) => grpc_service::tag::Tag::Value(v.clone()),
            }),
        }
    }
}
// protobuf types are really painful
impl From<&grpc_service::Tag> for Tag {
    fn from(t: &grpc_service::Tag) -> Self {
        match t.tag.as_ref().unwrap() {
            grpc_service::tag::Tag::Value(v) => Tag::Value(v.clone()),
            grpc_service::tag::Tag::ValueMap(m) => Tag::Map(
                m.values
                    .iter()
                    .map(|(k, tag)| (k.clone(), tag.into()))
                    .collect(),
            ),
            grpc_service::tag::Tag::ValueList(l) => {
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

impl QueryMatcher for Tag {
    fn qmatches(&self, query: &Query) -> bool {
        match self {
            Tag::Map(map) => map.qmatches(query),
            Tag::List(list) => list.qmatches(query),
            Tag::Value(v) => v.qmatches(query),
        }
    }
}

impl QueryMatcher for ExecutorMeta {
    fn qmatches(&self, query: &Query) -> bool {
        self.client_id.qmatches(query) || self.tags.qmatches(query)
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
            self.qmatches(&parse(query).unwrap())
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
    }
}
