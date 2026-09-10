from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text(encoding="utf-8")
    if text.count(old) != 1:
        raise SystemExit(f"expected exactly one match in {path}: {old[:100]!r}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8")


worker = "src/profile_worker.rs"
replace_once(
    worker,
    "        result: Result<PreparedProfile, ProfilePreparationError>,",
    "        result: Result<Box<PreparedProfile>, ProfilePreparationError>,",
)
replace_once(
    worker,
    "                let result = PreparedProfile::load(&intent);\n"
    "                ProfileWorkerCompletion::Prepared { selection, result }",
    "                let result = PreparedProfile::load(&intent).map(Box::new);\n"
    "                ProfileWorkerCompletion::Prepared { selection, result }",
)
replace_once(
    worker,
    "        let prepared = result.unwrap();\n"
    "        assert_eq!(prepared.selection(), selection);\n"
    "        runtime.commit_selection(prepared).unwrap();",
    "        let prepared = *result.unwrap();\n"
    "        assert_eq!(prepared.selection(), selection);\n"
    "        runtime.commit_selection(prepared).unwrap();",
)
replace_once(
    worker,
    "        let rejection = runtime.commit_selection(result.unwrap()).unwrap_err();\n"
    "        assert_eq!(\n"
    "            rejection.error(),\n"
    "            &ProfileRuntimeError::ActiveProfileNotDurable { profile: first }",
    "        let rejection = runtime.commit_selection(*result.unwrap()).unwrap_err();\n"
    "        assert_eq!(\n"
    "            rejection.error(),\n"
    "            &ProfileRuntimeError::ActiveProfileNotDurable { profile: first }",
)
replace_once(
    worker,
    "        let rejection = runtime.commit_selection(result.unwrap()).unwrap_err();\n"
    "        assert!(matches!(",
    "        let rejection = runtime.commit_selection(*result.unwrap()).unwrap_err();\n"
    "        assert!(matches!(",
)
replace_once(
    worker,
    "        runtime.commit_selection(result.unwrap()).unwrap();\n    }\n}",
    "        runtime.commit_selection(*result.unwrap()).unwrap();\n    }\n}",
)

windows = "src/platform/windows.rs"
replace_once(
    windows,
    "                let prepared = match result {\n"
    "                    Ok(prepared) => prepared,\n"
    "                    Err(error) => {",
    "                let prepared = match result {\n"
    "                    Ok(prepared) => *prepared,\n"
    "                    Err(error) => {",
)
