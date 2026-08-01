use super::{c::C, cpp::Cpp, javascript::JavaScript, python::Python, Language};
use crate::api::models::response::LanguageInfo;
use std::collections::HashMap;
use std::sync::Arc;

pub struct LanguageRegistry {
    languages: HashMap<String, Arc<dyn Language>>,
}

impl LanguageRegistry {
    pub fn build() -> Self {
        let mut r = Self {
            languages: HashMap::new(),
        };
        r.register(C);
        r.register(Cpp);
        r.register(Python);
        r.register(JavaScript);
        r
    }

    pub fn register(&mut self, lang: impl Language + 'static) {
        self.languages.insert(lang.id().to_string(), Arc::new(lang));
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Language>> {
        self.languages.get(id).cloned()
    }

    pub fn list(&self) -> Vec<LanguageInfo> {
        let mut list: Vec<LanguageInfo> = self
            .languages
            .values()
            .map(|l| LanguageInfo {
                id: l.id().to_string(),
                name: l.name().to_string(),
                version: l.version().to_string(),
            })
            .collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }
}
