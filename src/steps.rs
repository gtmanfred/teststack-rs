use indexmap::IndexMap;
use toml::Value;

#[derive(Debug, Clone, Default)]
pub struct Step {
    pub name: String,
    pub command: Option<StepCommand>,
    pub check: Option<StepCommand>,
    pub user: Option<String>,
    pub requires: Vec<String>,
    pub required_by: Vec<String>,
    pub exit_code: Option<i64>,
    pub check_exit_code: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum StepCommand {
    One(String),
    Many(Vec<String>),
}

impl StepCommand {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            StepCommand::One(s) => vec![s.clone()],
            StepCommand::Many(v) => v.clone(),
        }
    }
}

pub fn process_steps(table: IndexMap<String, Value>) -> IndexMap<String, Step> {
    let mut out: IndexMap<String, Step> = IndexMap::new();
    for (name, raw) in table {
        let mut s = out.remove(&name).unwrap_or_default();
        s.name = name.clone();
        match raw {
            Value::String(c) => s.command = Some(StepCommand::One(c)),
            Value::Array(arr) => {
                s.command = Some(StepCommand::Many(
                    arr.into_iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                ));
            }
            Value::Table(t) => {
                if let Some(c) = t.get("command") {
                    s.command = parse_cmd(c);
                }
                if let Some(c) = t.get("check") {
                    s.check = parse_cmd(c);
                }
                if let Some(u) = t.get("user").and_then(|v| v.as_str()) {
                    s.user = Some(u.into());
                }
                if let Some(reqs) = t.get("requires").and_then(|v| v.as_array()) {
                    for r in reqs {
                        if let Some(rn) = r.as_str() {
                            s.requires.push(rn.into());
                            let entry = out.entry(rn.into()).or_default();
                            entry.name = rn.into();
                            entry.required_by.push(name.clone());
                        }
                    }
                }
            }
            _ => {}
        }
        out.insert(name, s);
    }
    out
}

fn parse_cmd(v: &Value) -> Option<StepCommand> {
    match v {
        Value::String(s) => Some(StepCommand::One(s.clone())),
        Value::Array(arr) => Some(StepCommand::Many(
            arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        )),
        _ => None,
    }
}

pub trait StepRunner {
    fn run(&mut self, command: &str, user: Option<&str>) -> i64;
}

pub fn run_all<R: StepRunner>(steps: &mut IndexMap<String, Step>, runner: &mut R, posargs: &[String]) -> i64 {
    let names: Vec<String> = steps.keys().cloned().collect();
    for n in names {
        let code = run_step(&n, steps, runner, posargs);
        steps.get_mut(&n).unwrap().exit_code = Some(code);
    }
    steps.values().map(|s| s.exit_code.unwrap_or(0)).sum()
}

fn run_step<R: StepRunner>(
    name: &str,
    steps: &mut IndexMap<String, Step>,
    runner: &mut R,
    posargs: &[String],
) -> i64 {
    if let Some(c) = steps.get(name).and_then(|s| s.exit_code) {
        return c;
    }

    let has_check = steps.get(name).map_or(false, |s| s.check.is_some());
    if has_check {
        let cc = do_check(name, steps, runner, posargs);
        steps.get_mut(name).unwrap().check_exit_code = Some(cc);
        if cc == 0 {
            return 0;
        }
    }

    let required_by: Vec<String> = steps.get(name).map(|s| s.required_by.clone()).unwrap_or_default();
    if !required_by.is_empty() {
        let mut any_needed = false;
        for rb in &required_by {
            if steps.get(rb).and_then(|s| s.exit_code).is_some() {
                continue;
            }
            let cc = do_check(rb, steps, runner, posargs);
            if cc != 0 {
                any_needed = true;
                break;
            }
        }
        if !any_needed {
            return 0;
        }
    }

    let requires: Vec<String> = steps.get(name).map(|s| s.requires.clone()).unwrap_or_default();
    let mut req_code = 0i64;
    for r in &requires {
        let c = run_step(r, steps, runner, posargs);
        steps.get_mut(r).unwrap().exit_code = Some(c);
        req_code += c;
    }
    if req_code != 0 {
        return req_code;
    }

    let cmd = steps.get(name).and_then(|s| s.command.clone());
    let user = steps.get(name).and_then(|s| s.user.clone());
    let Some(cmd) = cmd else {
        return 0;
    };
    run_cmd(&cmd, user.as_deref(), runner, posargs)
}

fn do_check<R: StepRunner>(
    name: &str,
    steps: &mut IndexMap<String, Step>,
    runner: &mut R,
    posargs: &[String],
) -> i64 {
    if let Some(c) = steps.get(name).and_then(|s| s.check_exit_code) {
        return c;
    }
    let required_by: Vec<String> = steps.get(name).map(|s| s.required_by.clone()).unwrap_or_default();
    if !required_by.is_empty() {
        let mut code = 1i64;
        for rb in &required_by {
            code = do_check(rb, steps, runner, posargs);
        }
        if code == 0 {
            return code;
        }
    }
    let check = steps.get(name).and_then(|s| s.check.clone());
    if let Some(c) = check {
        let user = steps.get(name).and_then(|s| s.user.clone());
        let code = run_cmd(&c, user.as_deref(), runner, posargs);
        steps.get_mut(name).unwrap().check_exit_code = Some(code);
        return code;
    }
    1
}

fn run_cmd<R: StepRunner>(cmd: &StepCommand, user: Option<&str>, runner: &mut R, posargs: &[String]) -> i64 {
    let joined = posargs.join(" ");
    let mut total = 0i64;
    for c in cmd.to_vec() {
        let rendered = c.replace("{posargs}", &joined);
        total += runner.run(&rendered, user);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Fake {
        pub log: Vec<String>,
        pub map: BTreeMap<String, i64>,
    }
    impl StepRunner for Fake {
        fn run(&mut self, command: &str, _user: Option<&str>) -> i64 {
            self.log.push(command.into());
            *self.map.get(command).unwrap_or(&0)
        }
    }

    #[test]
    fn dag_skips_on_check_zero() {
        let toml = r#"
            [steps]
            install = "pip install"
            [steps.env]
            command = "env"
            check = "test -f /a"
        "#;
        let v: Value = toml::from_str(toml).unwrap();
        let t = v.get("steps").unwrap().as_table().unwrap().clone();
        let map: IndexMap<String, Value> = t.into_iter().collect();
        let mut steps = process_steps(map);
        let mut runner = Fake {
            log: vec![],
            map: BTreeMap::from([("test -f /a".to_string(), 0i64)]),
        };
        run_all(&mut steps, &mut runner, &[]);
        assert!(runner.log.contains(&"test -f /a".into()));
        assert!(!runner.log.contains(&"env".into()));
    }

    #[test]
    fn preserves_toml_order() {
        let toml = r#"
            [steps]
            install = "pip install"
            database = "migrate"
            flake8 = "flake8"
            tests = "pytest"
        "#;
        let v: Value = toml::from_str(toml).unwrap();
        let t = v.get("steps").unwrap().as_table().unwrap().clone();
        let map: IndexMap<String, Value> = t.into_iter().collect();
        let mut steps = process_steps(map);
        let mut runner = Fake { log: vec![], map: BTreeMap::new() };
        run_all(&mut steps, &mut runner, &[]);
        assert_eq!(runner.log, vec!["pip install", "migrate", "flake8", "pytest"]);
    }

    #[test]
    fn requires_runs_first() {
        let toml = r#"
            [steps.touch]
            command = "cp a b"
            requires = ["env"]
            [steps.env]
            command = "env"
        "#;
        let v: Value = toml::from_str(toml).unwrap();
        let t = v.get("steps").unwrap().as_table().unwrap().clone();
        let map: IndexMap<String, Value> = t.into_iter().collect();
        let mut steps = process_steps(map);
        let mut runner = Fake { log: vec![], map: BTreeMap::new() };
        run_all(&mut steps, &mut runner, &[]);
        let env_pos = runner.log.iter().position(|c| c == "env").unwrap();
        let touch_pos = runner.log.iter().position(|c| c == "cp a b").unwrap();
        assert!(env_pos < touch_pos);
    }
}
