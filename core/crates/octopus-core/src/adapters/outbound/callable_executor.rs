use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::domain::ports::{ActivityFunc, IActivityExecutor};
use crate::json::ValueMap;

/// Runs any activity that has no handler of its own: a host that keeps its
/// own activity registry (an SDK's executor) answers for it.
#[derive(Clone)]
pub struct ActivityResolver {
    pub call: ResolveCall,
    pub has: ResolveHas,
}

/// Runs the named activity.
pub type ResolveCall = Arc<dyn Fn(&str, &ValueMap) -> Result<ValueMap, String> + Send + Sync>;
/// Whether the host knows the named activity.
pub type ResolveHas = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub struct CallableActivityExecutor {
    activities: RwLock<HashMap<String, ActivityFunc>>,
    resolver: RwLock<Option<ActivityResolver>>,
}

impl CallableActivityExecutor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Handlers registered by name still come first.
    pub fn set_resolver(&self, resolver: Option<ActivityResolver>) {
        *self.resolver.write().unwrap() = resolver;
    }

    pub fn register<F>(&self, name: &str, handler: F)
    where
        F: Fn(&ValueMap) -> Result<ValueMap, String> + Send + Sync + 'static,
    {
        self.register_activity(name, Arc::new(handler));
    }
}

impl Default for CallableActivityExecutor {
    fn default() -> Self {
        Self {
            activities: RwLock::new(HashMap::new()),
            resolver: RwLock::new(None),
        }
    }
}

impl IActivityExecutor for CallableActivityExecutor {
    fn execute_activity(&self, name: &str, input: &ValueMap) -> Result<ValueMap, String> {
        let handler = {
            let guard = self.activities.read().unwrap();
            guard.get(name).cloned()
        };
        let Some(handler) = handler else {
            let resolver = self.resolver.read().unwrap().clone();
            if let Some(r) = resolver {
                return (r.call)(name, input);
            }
            return Err(format!(
                "activity handler '{}' not registered in Live Engine",
                name
            ));
        };
        handler(input)
    }

    fn register_activity(&self, name: &str, handler: ActivityFunc) {
        self.activities
            .write()
            .unwrap()
            .insert(name.to_string(), handler);
    }

    fn has_activity(&self, name: &str) -> bool {
        if self.activities.read().unwrap().contains_key(name) {
            return true;
        }
        let resolver = self.resolver.read().unwrap().clone();
        resolver.is_some_and(|r| (r.has)(name))
    }

    fn list_activities(&self) -> Vec<String> {
        self.activities.read().unwrap().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmap;

    #[test]
    fn the_resolver_answers_for_unregistered_activities() {
        let ex = CallableActivityExecutor::new();
        ex.register("Own", |_: &ValueMap| Ok(vmap! { "by" => "handler" }));
        assert!(!ex.has_activity("Other"));
        ex.set_resolver(Some(ActivityResolver {
            call: Arc::new(|name, _| Ok(vmap! { "by" => name })),
            has: Arc::new(|name| name == "Other"),
        }));
        assert!(ex.has_activity("Other") && !ex.has_activity("Missing"));
        assert_eq!(
            ex.execute_activity("Own", &ValueMap::new()).unwrap()["by"],
            "handler".into()
        );
        assert_eq!(
            ex.execute_activity("Other", &ValueMap::new()).unwrap()["by"],
            "Other".into()
        );
    }
}
