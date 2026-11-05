use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::domain::models::WorkflowBlueprint;
use crate::domain::ports::IBlueprintRepository;

/// Blueprints by workflow name, and the names in registration order.
type Registry = (HashMap<String, Arc<WorkflowBlueprint>>, Vec<String>);

pub struct YamlBlueprintRepository {
    blueprints: RwLock<Registry>,
}

impl YamlBlueprintRepository {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            blueprints: RwLock::new((HashMap::new(), Vec::new())),
        })
    }

    pub fn load_from_directory(&self, dir_path: &str) -> Result<(), String> {
        let entries = fs::read_dir(dir_path).map_err(|e| e.to_string())?;
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext == "yaml" || ext == "yml" {
                paths.push(path);
            }
        }
        paths.sort();

        for file_path in paths {
            let content = fs::read_to_string(&file_path)
                .map_err(|e| format!("reading blueprint {}: {}", file_path.display(), e))?;
            let bp = WorkflowBlueprint::from_yaml_str(&content)
                .map_err(|e| format!("parsing blueprint {}: {}", file_path.display(), e))?;
            if bp.workflow_name.is_empty() {
                return Err(format!(
                    "blueprint {}: missing workflow_name",
                    file_path.display()
                ));
            }
            self.save_blueprint(bp)?;
        }
        Ok(())
    }

    pub fn load_from_file(&self, file_path: impl AsRef<Path>) -> Result<WorkflowBlueprint, String> {
        let path = file_path.as_ref();
        let content = fs::read_to_string(path)
            .map_err(|e| format!("reading blueprint {}: {}", path.display(), e))?;
        let bp = WorkflowBlueprint::from_yaml_str(&content)
            .map_err(|e| format!("parsing blueprint {}: {}", path.display(), e))?;
        if bp.workflow_name.is_empty() {
            return Err(format!(
                "blueprint {}: missing workflow_name",
                path.display()
            ));
        }
        self.save_blueprint(bp.clone())?;
        Ok(bp)
    }
}

impl Default for YamlBlueprintRepository {
    fn default() -> Self {
        Self {
            blueprints: RwLock::new((HashMap::new(), Vec::new())),
        }
    }
}

impl IBlueprintRepository for YamlBlueprintRepository {
    fn get_blueprint(&self, workflow_name: &str) -> Result<Arc<WorkflowBlueprint>, String> {
        let guard = self.blueprints.read().unwrap();
        guard
            .0
            .get(workflow_name)
            .cloned()
            .ok_or_else(|| format!("blueprint '{}' not found", workflow_name))
    }

    fn list_blueprints(&self) -> Result<Vec<WorkflowBlueprint>, String> {
        let guard = self.blueprints.read().unwrap();
        let mut list = Vec::with_capacity(guard.1.len());
        for name in &guard.1 {
            if let Some(bp) = guard.0.get(name) {
                list.push(WorkflowBlueprint::clone(bp));
            }
        }
        Ok(list)
    }

    fn save_blueprint(&self, mut blueprint: WorkflowBlueprint) -> Result<(), String> {
        blueprint.validate()?;
        crate::domain::services::validate_blueprint_conditions(&blueprint)?;
        blueprint.normalize_endpoints();
        if blueprint.steps.is_empty()
            && !blueprint.api.is_empty()
            && !blueprint.api[0].steps.is_empty()
        {
            blueprint.steps = blueprint.api[0].steps.clone();
        }
        let mut guard = self.blueprints.write().unwrap();
        let wf_name = blueprint.workflow_name.clone();
        if !guard.0.contains_key(&wf_name) {
            guard.1.push(wf_name.clone());
        }
        guard.0.insert(wf_name, Arc::new(blueprint));
        Ok(())
    }
}
