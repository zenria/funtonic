use crate::config::ExecutorConfig;
use crate::executor_meta::ColonSplitMatch::Colon;
use crate::VERSION;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Tag {
    Map(HashMap<String, String>),
    List(Vec<String>),
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

impl Tag {
    pub fn matches(&self, pattern: &str) -> bool {
        if pattern == "*" {
            true
        } else {
            match self {
                Tag::Value(v) => v == pattern,
                Tag::List(v) => v.iter().any(|v| v == pattern),
                Tag::Map(v) => match colon_split(pattern, v) {
                    ColonSplitMatch::NoColon => v.iter().any(|(_, v)| v == pattern),
                    Colon(sub_pattern, matching_value) => {
                        sub_pattern == "*" || matching_value.map_or(false, |v| sub_pattern == v)
                    }
                },
            }
        }
    }
}

impl ExecutorMeta {
    pub fn matches(&self, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        // tag match
        if let ColonSplitMatch::Colon(sub_pattern, matching_tag) = colon_split(pattern, &self.tags)
        {
            return matching_tag.map_or(false, |tag| tag.matches(sub_pattern));
        }
        // client id match
        self.client_id == pattern
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn tags(&self) -> &HashMap<String, Tag> {
        &self.tags
    }
}

enum ColonSplitMatch<'a, 'p, T> {
    NoColon,
    Colon(&'p str, Option<&'a T>),
}

fn colon_split<'a, 'p, T>(
    pattern: &'p str,
    map: &'a HashMap<String, T>,
) -> ColonSplitMatch<'a, 'p, T> {
    if pattern.contains(":") {
        let split: Vec<&str> = pattern.splitn(2, ":").collect();
        let key = split[0];
        ColonSplitMatch::Colon(split[1], map.get(key))
    } else {
        ColonSplitMatch::NoColon
    }
}

#[cfg(test)]
mod test {
    use crate::executor_meta::{ExecutorMeta, Tag};
    use std::collections::HashMap;

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

        let mut maap = HashMap::new();
        maap.insert(String::from("key1"), String::from("value1"));
        maap.insert(String::from("key2"), String::from("value2"));
        let maap = Tag::Map(maap);
        assert!(maap.matches("*"));
        assert!(maap.matches("value1"));
        assert!(maap.matches("value2"));
        assert!(!maap.matches("value3"));
        assert!(maap.matches("key1:value1"));
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

        serde_yaml::from_str::<HashMap<String, Tag>>("env: prod\nroles:\n  - foo\n  - bar\nos:\n  type: Linux\n  sub_type: Ubuntu\n  version: \"18.04\"")
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
        assert!(!meta.matches("env:"));
        assert!(meta.matches("env:prod"));
        assert!(!meta.matches("env:dev"));
        assert!(!meta.matches("roles:"));
        assert!(meta.matches("roles:*"));
        assert!(meta.matches("roles:foo"));
        assert!(meta.matches("roles:bar"));
        assert!(!meta.matches("non_existing:bar"));
        assert!(!meta.matches("non_existing:*"));

        assert!(meta.matches("os:*"));
        assert!(meta.matches("os:Linux"));
        assert!(meta.matches("os:type:Linux"));
        assert!(meta.matches("os:type:*"));
        assert!(meta.matches("os:version:18.04"));
        assert!(!meta.matches("os:type:Windows"));
    }
}
