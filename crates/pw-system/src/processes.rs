//! Turning the process table into the few rows worth showing, and totalling tracked trees.
//!
//! Pure functions over [`Row`]s, so they're tested without a live system.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::sample::{Process, Tree};

/// How many processes each "top" list keeps.
pub(crate) const TOP: usize = 5;
/// GPU processes are all worth naming, up to this many.
const MAX_GPU: usize = 8;
/// A parent chain longer than this is treated as unrelated (it can't be real; it's a race).
const MAX_DEPTH: usize = 64;

/// One process from the OS table.
#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub pid: u32,
    pub parent: Option<u32>,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
    pub gpu_memory: Option<u64>,
}

/// Which tracked root each process descends from (roots map to themselves).
pub(crate) fn attribute(rows: &[Row], roots: &[u32]) -> HashMap<u32, u32> {
    let parents: HashMap<u32, Option<u32>> = rows.iter().map(|r| (r.pid, r.parent)).collect();
    let roots: HashSet<u32> = roots.iter().copied().collect();
    // `None` caches "descends from no root" so every chain is walked once.
    let mut memo: HashMap<u32, Option<u32>> = HashMap::new();
    let mut owners = HashMap::new();
    for row in rows {
        let mut chain = Vec::new();
        let mut pid = Some(row.pid);
        let found = loop {
            let Some(p) = pid else { break None };
            if roots.contains(&p) {
                break Some(p);
            }
            if let Some(&known) = memo.get(&p) {
                break known;
            }
            if chain.len() >= MAX_DEPTH {
                break None;
            }
            chain.push(p);
            pid = parents.get(&p).copied().flatten().filter(|&parent| parent != p);
        };
        for p in chain {
            memo.insert(p, found);
        }
        if let Some(root) = found {
            owners.insert(row.pid, root);
        }
    }
    owners
}

/// Totals per root, in the order the roots were given. Roots with no live process are left out.
pub(crate) fn trees(rows: &[Row], owners: &HashMap<u32, u32>, roots: &[u32]) -> Vec<Tree> {
    let mut totals: HashMap<u32, Tree> = HashMap::new();
    for row in rows {
        let Some(&root) = owners.get(&row.pid) else { continue };
        let tree = totals.entry(root).or_insert(Tree { root, cpu: 0.0, memory: 0, gpu_memory: 0, processes: 0 });
        tree.cpu += row.cpu;
        tree.memory += row.memory;
        tree.gpu_memory += row.gpu_memory.unwrap_or(0);
        tree.processes += 1;
    }
    roots.iter().filter_map(|r| totals.remove(r)).collect()
}

/// The top few by CPU and by memory, plus GPU users, without duplicates.
///
/// Processes with the same name under the same root are one entry (a browser's dozens of
/// helpers are one "chrome"), so the lists name programs rather than repeat one.
pub(crate) fn notable(rows: &[Row], owners: &HashMap<u32, u32>) -> Vec<Process> {
    let mut groups: HashMap<(&str, Option<u32>), Process> = HashMap::new();
    for row in rows {
        let root = owners.get(&row.pid).copied();
        let group = groups.entry((row.name.as_str(), root)).or_insert_with(|| Process {
            pid: row.pid,
            name: row.name.clone(),
            count: 0,
            cpu: 0.0,
            memory: 0,
            gpu_memory: None,
            root,
        });
        // The lowest pid is usually the one that started the rest.
        group.pid = group.pid.min(row.pid);
        group.count += 1;
        group.cpu += row.cpu;
        group.memory += row.memory;
        if let Some(bytes) = row.gpu_memory {
            *group.gpu_memory.get_or_insert(0) += bytes;
        }
    }
    let groups: Vec<Process> = groups.into_values().collect();

    fn take(picked: &mut Vec<Process>, groups: &[Process], key: impl Fn(&Process) -> f64, n: usize) {
        let mut candidates: Vec<&Process> = groups.iter().filter(|g| key(g) > 0.0).collect();
        candidates.sort_by(|a, b| key(b).total_cmp(&key(a)).then_with(|| a.name.cmp(&b.name)));
        for group in candidates.into_iter().take(n) {
            if !picked.iter().any(|p| p.name == group.name && p.root == group.root) {
                picked.push(group.clone());
            }
        }
    }
    let mut picked = Vec::new();
    take(&mut picked, &groups, |g| f64::from(g.cpu), TOP);
    take(&mut picked, &groups, |g| g.memory as f64, TOP);
    take(&mut picked, &groups, |g| g.gpu_memory.unwrap_or(0) as f64, MAX_GPU);
    picked
}

/// Programs that run a script: the script says more than the program does.
const RUNTIMES: &[&str] = &["node", "nodejs", "bun", "deno", "python", "python2", "python3", "ruby", "perl", "pypy3"];
/// Entry-point file names that say nothing; their folder is the better name.
const GENERIC: &[&str] = &["index", "main", "cli", "__main__", "run", "app", "server", "entry", "start"];
/// Folders that say nothing either.
const PLUMBING: &[&str] = &["bin", "dist", "lib", "build", "src", "out", "node_modules", ".bin", "scripts"];

/// A readable name for a process: `claude` for `node …/claude-code/cli.js`, the full program
/// name where Linux cut it to 15 characters, and `bash` for a login `-bash`.
pub(crate) fn display_name(short: &str, cmd: &[String]) -> String {
    let short = short.trim_start_matches('-');
    // Some programs (Chrome, Electron) rewrite their whole command line into the first argument.
    let program = cmd.first().and_then(|c| c.split_whitespace().next()).map(|c| c.trim_start_matches('-'));
    let program = program.and_then(|p| Path::new(p).file_name()?.to_str()).filter(|p| !p.is_empty());
    let base = match program {
        // The kernel's name is cut at 15 characters; the command line isn't.
        Some(p) if short.len() >= 15 && p.starts_with(short) => p,
        Some(p) if p == short || short.is_empty() => p,
        _ => short,
    };
    let runtime = base.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if (RUNTIMES.contains(&base) || RUNTIMES.contains(&runtime))
        && let Some(script) = script_name(&cmd[cmd.len().min(1)..])
    {
        return script;
    }
    base.to_owned()
}

fn script_name(args: &[String]) -> Option<String> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            // `python -m http.server`
            "-m" => return args.next().cloned(),
            // `deno run main.ts`, `bun run dev`
            "run" | "x" | "exec" => continue,
            a if a.starts_with('-') => continue,
            a => return Some(script_label(Path::new(a))),
        }
    }
    None
}

fn script_label(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    if !GENERIC.contains(&stem) && !stem.is_empty() {
        return stem.to_owned();
    }
    let folder = path
        .ancestors()
        .skip(1)
        .filter_map(|a| a.file_name()?.to_str())
        .find(|name| !PLUMBING.contains(name) && !name.starts_with('@'));
    folder.unwrap_or(stem).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, parent: Option<u32>, cpu: f32, memory: u64) -> Row {
        Row { pid, parent, name: format!("p{pid}"), cpu, memory, gpu_memory: None }
    }

    fn cmd(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn descendants_belong_to_their_root() {
        // 1 ─┬─ 10 (root) ─ 11 ─ 12
        //    ├─ 20 (root)
        //    └─ 30 ─ 31
        let rows = vec![
            row(1, None, 0.0, 0),
            row(10, Some(1), 1.0, 100),
            row(11, Some(10), 2.0, 200),
            row(12, Some(11), 3.0, 300),
            row(20, Some(1), 0.5, 50),
            row(30, Some(1), 9.0, 900),
            row(31, Some(30), 9.0, 900),
        ];
        let owners = attribute(&rows, &[10, 20, 99]);
        assert_eq!(owners.get(&12), Some(&10));
        assert_eq!(owners.get(&10), Some(&10));
        assert_eq!(owners.get(&20), Some(&20));
        assert_eq!(owners.get(&31), None);
        assert_eq!(owners.get(&1), None);

        let trees = trees(&rows, &owners, &[10, 20, 99]);
        assert_eq!(trees.len(), 2, "a root that isn't running has no tree");
        assert_eq!((trees[0].root, trees[0].cpu, trees[0].memory, trees[0].processes), (10, 6.0, 600, 3));
        assert_eq!((trees[1].root, trees[1].processes), (20, 1));
    }

    #[test]
    fn parent_loops_end() {
        let rows = vec![row(5, Some(6), 0.0, 0), row(6, Some(5), 0.0, 0), row(7, Some(7), 0.0, 0)];
        assert!(attribute(&rows, &[1]).is_empty());
    }

    #[test]
    fn notable_takes_the_top_of_each_list_once() {
        let mut rows: Vec<Row> = (1..=20).map(|i| row(i, None, i as f32, u64::from(i) * 10)).collect();
        rows[0].gpu_memory = Some(4096);
        rows.push(row(99, None, 0.0, 0));
        let picked = notable(&rows, &HashMap::from([(20, 7)]));
        let pids: Vec<u32> = picked.iter().map(|p| p.pid).collect();
        // Top CPU and top memory are the same five here; the GPU user joins them.
        assert_eq!(pids, vec![20, 19, 18, 17, 16, 1]);
        assert_eq!(picked[0].root, Some(7));
        assert!(!pids.contains(&99), "idle processes are never listed");
    }

    #[test]
    fn same_named_processes_are_one_program_per_root() {
        let named = |pid, name: &str, cpu, memory| Row { name: name.into(), ..row(pid, None, cpu, memory) };
        let rows = vec![
            named(30, "chrome", 1.0, 100),
            named(31, "chrome", 2.0, 300),
            named(32, "chrome", 0.0, 50),
            named(40, "claude", 4.0, 400),
            named(41, "claude", 1.0, 200),
            named(50, "Xorg", 0.5, 10),
        ];
        // The two `claude`s run in different workspaces.
        let picked = notable(&rows, &HashMap::from([(40, 1), (41, 2)]));
        let chrome = picked.iter().find(|p| p.name == "chrome").unwrap();
        assert_eq!((chrome.pid, chrome.count, chrome.cpu, chrome.memory), (30, 3, 3.0, 450));
        let claudes: Vec<_> = picked.iter().filter(|p| p.name == "claude").map(|p| (p.root, p.count)).collect();
        assert_eq!(claudes, vec![(Some(1), 1), (Some(2), 1)]);
        assert_eq!(picked[0].name, "claude", "busiest first");
    }

    #[test]
    fn names_read_naturally() {
        assert_eq!(display_name("node", &cmd(&["node", "/usr/lib/node_modules/@openai/codex/bin/codex.js"])), "codex");
        assert_eq!(
            display_name("node", &cmd(&["node", "--no-warnings", "/opt/claude-code/cli.js", "--resume"])),
            "claude-code"
        );
        assert_eq!(display_name("python3", &cmd(&["python3", "-m", "http.server"])), "http.server");
        assert_eq!(display_name("python3.12", &cmd(&["/usr/bin/python3.12", "train.py"])), "train");
        assert_eq!(display_name("bun", &cmd(&["bun", "run", "dev"])), "dev");
        assert_eq!(display_name("node", &cmd(&["node"])), "node");
        assert_eq!(display_name("-bash", &cmd(&["-bash"])), "bash");
        assert_eq!(
            display_name("gnome-shell-cal", &cmd(&["/usr/libexec/gnome-shell-calendar-server"])),
            "gnome-shell-calendar-server"
        );
        assert_eq!(display_name("chrome", &cmd(&["/opt/google/chrome/chrome --type=renderer --lang=en"])), "chrome");
        // A program that renamed itself keeps the name it chose.
        assert_eq!(display_name("postgres", &cmd(&["postgres: checkpointer"])), "postgres");
        assert_eq!(display_name("kworker/0:1", &[]), "kworker/0:1");
    }
}
