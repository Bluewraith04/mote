use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use compiler::ast::Item;
use crate::path::CanonicalModuleId;
use crate::resolver::ModuleResolver;

#[derive(Clone, Debug)]
/// One module in the import graph.
pub struct ModuleNode {
    pub id: CanonicalModuleId,
    pub target: PathBuf,
    pub dependencies: Vec<CanonicalModuleId>,
}

#[derive(Clone, Debug)]
/// The import graph between modules.
pub struct DependencyGraph {
    pub nodes: HashMap<CanonicalModuleId, ModuleNode>,
    pub entry_id: CanonicalModuleId,
    /// Modules in DFS post-order: each appears after every module it imports, in the order its own imports appear.
    pub discovery_order: Vec<CanonicalModuleId>,
}

impl DependencyGraph {
    pub fn build(entry_path: PathBuf, resolver: &mut ModuleResolver) -> Result<Self, String> {
        let entry_target = entry_path.clone();
        let entry_canonical = entry_path.canonicalize().unwrap_or(entry_path.clone()).to_string_lossy().to_string();
        let entry_id = CanonicalModuleId::new(entry_canonical);

        let mut graph = Self {
            nodes: HashMap::new(),
            entry_id: entry_id.clone(),
            discovery_order: Vec::new(),
        };

        let mut visited = HashSet::new();
        graph.traverse_and_build(entry_id, entry_target, resolver, &mut visited)?;

        if let Some(cycle) = graph.detect_cycles() {
            return Err(format!("Circular import detected: {}", graph.format_cycle(&cycle)));
        }

        Ok(graph)
    }

    fn traverse_and_build(
        &mut self,
        current_id: CanonicalModuleId,
        current_target: PathBuf,
        resolver: &mut ModuleResolver,
        visited: &mut HashSet<CanonicalModuleId>,
    ) -> Result<(), String> {
        if visited.contains(&current_id) {
            return Ok(());
        }
        visited.insert(current_id.clone());

        let mut deps = Vec::new();

        {
            let path = &current_target;
            let program = resolver.parse_module(&current_id, path)?;
            let current_dir = path.parent().unwrap_or(Path::new("."));

            for item in &program.items {
                if let Item::Import(ref imp) = item {
                    let (dep_id, dep_target) = resolver.resolve_path(&imp.path, current_dir)?;

                    deps.push(dep_id.clone());

                    if !visited.contains(&dep_id) {
                        self.traverse_and_build(dep_id, dep_target, resolver, visited)?;
                    }
                }
            }
        }

        self.discovery_order.push(current_id.clone());
        self.nodes.insert(
            current_id.clone(),
            ModuleNode {
                id: current_id,
                target: current_target,
                dependencies: deps,
            },
        );

        Ok(())
    }

    /// Finds an import cycle by DFS and returns the first one found.
    pub(crate) fn detect_cycles(&self) -> Option<Vec<CanonicalModuleId>> {
        let mut visited = HashSet::new();
        let mut rec_stack = Vec::new();

        for node_id in self.nodes.keys() {
            if let Some(cycle) = self.dfs_cycle(node_id, &mut visited, &mut rec_stack) {
                return Some(cycle);
            }
        }
        None
    }

    fn format_cycle(&self, cycle: &[CanonicalModuleId]) -> String {
        let mut name_counts: HashMap<&str, usize> = HashMap::new();
        for id in self.nodes.keys() {
            *name_counts.entry(id.display_name()).or_insert(0) += 1;
        }
        cycle
            .iter()
            .map(|id| {
                if name_counts.get(id.display_name()).copied().unwrap_or(0) > 1 {
                    id.as_str().to_string()
                } else {
                    id.display_name().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    fn dfs_cycle(
        &self,
        node_id: &CanonicalModuleId,
        visited: &mut HashSet<CanonicalModuleId>,
        rec_stack: &mut Vec<CanonicalModuleId>,
    ) -> Option<Vec<CanonicalModuleId>> {
        if let Some(pos) = rec_stack.iter().position(|x| x == node_id) {
            let mut cycle = rec_stack[pos..].to_vec();
            cycle.push(node_id.clone());
            return Some(cycle);
        }

        if visited.contains(node_id) {
            return None;
        }
        visited.insert(node_id.clone());
        rec_stack.push(node_id.clone());

        if let Some(node) = self.nodes.get(node_id) {
            for dep in &node.dependencies {
                if let Some(cycle) = self.dfs_cycle(dep, visited, rec_stack) {
                    return Some(cycle);
                }
            }
        }

        rec_stack.pop();
        None
    }

    /// A dependency-first topological sort of the modules; ties between ready modules are broken by `discovery_order`, so the init order is stable.
    pub fn topological_sort(&self) -> Result<Vec<CanonicalModuleId>, String> {
        let discovery_index: HashMap<&CanonicalModuleId, usize> = self
            .discovery_order
            .iter()
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();
        let rank = |id: &CanonicalModuleId| discovery_index.get(id).copied().unwrap_or(usize::MAX);

        let mut in_degrees: HashMap<CanonicalModuleId, usize> = HashMap::new();
        let mut dependents: HashMap<CanonicalModuleId, Vec<CanonicalModuleId>> = HashMap::new();

        for node_id in self.nodes.keys() {
            in_degrees.insert(node_id.clone(), 0);
            dependents.insert(node_id.clone(), Vec::new());
        }

        for (node_id, node) in &self.nodes {
            for dep in &node.dependencies {
                if let Some(list) = dependents.get_mut(dep) {
                    list.push(node_id.clone());
                }
                *in_degrees.entry(node_id.clone()).or_insert(0) += 1;
            }
        }

        let mut ready: Vec<CanonicalModuleId> = in_degrees
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(id, _)| id.clone())
            .collect();
        ready.sort_by_key(|id| std::cmp::Reverse(rank(id)));

        let mut order = Vec::new();
        while let Some(current) = ready.pop() {
            order.push(current.clone());
            if let Some(deps) = dependents.get(&current) {
                for dependent in deps {
                    if let Some(deg) = in_degrees.get_mut(dependent) {
                        *deg -= 1;
                        if *deg == 0 {
                            ready.push(dependent.clone());
                        }
                    }
                }
            }
            ready.sort_by_key(|id| std::cmp::Reverse(rank(id)));
        }

        if order.len() != self.nodes.len() {
            return Err("Cycle detected during topological sort".into());
        }

        Ok(order)
    }
}
