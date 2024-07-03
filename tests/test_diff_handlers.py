import json
import random
import requests

from pathlib import Path

from termcolor import colored

current_dir = Path(__file__).parent.absolute()
test_file = current_dir / "test_file.rs.temp"


file_text = """
pub async fn execute_at_file(ccx: &mut AtCommandsContext, file_path: String) -> Result<ContextFile, String> {
    let candidates = parameter_repair_candidates(&file_path, ccx).await;
    if candidates.is_empty() {
// my end0
        info!("parameter {:?} is uncorrectable :/", &file_path);
        return Err(format!("parameter {:?} is uncorrectable :/", &file_path));
    }
    // my start1
    let mut file_path = candidates.get(0).unwrap().clone();
    // my end1
    let mut line1 = 0;
    let mut line2 = 0;

    let colon_kind_mb = colon_lines_range_from_arg(&mut file_path);

    let gradient_type = gradient_type_from_range_kind(&colon_kind_mb);

    let cpath = crate::files_correction::canonical_path(&file_path);
    // my start2
    let file_text = get_file_text_from_memory_or_disk(ccx.global_context.clone(), &cpath).await?;
    if let Some(colon) = &colon_kind_mb {
        line1 = colon.line1;
        line2 = colon.line2;
    }
    // my end2
    if line1 == 0 && line2 == 0 {
        line2 = file_text.lines().count()
    }
    // my start3
    Ok(ContextFile {
        file_name: file_path.clone(),
        file_content: file_text,
        line1,
        line2,
        symbol: Uuid::default(),
        gradient_type,
        usefulness: 100.0,
        is_body_important: false
    })
    // my end3
    // my start4
}
    // my end4
"""[1:-1]

i0 = """
// insert0
"""[1:]

orig0 = """
pub async fn execute_at_file(ccx: &mut AtCommandsContext, file_path: String) -> Result<ContextFile, String> {
    let candidates = parameter_repair_candidates(&file_path, ccx).await;
    if candidates.is_empty() {
"""[1:]


i1 = ""

orig1 = """
    let mut file_path = candidates.get(0).unwrap().clone();
"""[1:]

i2 = """
// insert2
// insert2
// insert2
"""[1:]

orig2 = """
    let file_text = get_file_text_from_memory_or_disk(ccx.global_context.clone(), &cpath).await?;
    if let Some(colon) = &colon_kind_mb {
        line1 = colon.line1;
        line2 = colon.line2;
    }
"""[1:]


i3 = """
// insert3
// insert3
// insert3
// insert3
"""[1:]

orig3 = """
    Ok(ContextFile {
        file_name: file_path.clone(),
        file_content: file_text,
        line1,
        line2,
        symbol: Uuid::default(),
        gradient_type,
        usefulness: 100.0,
        is_body_important: false
    })
"""[1:]

i4 = "//insert4\n"

orig4 = """
}
"""[1:]


text_after_apply = """
// insert0
// my end0
        info!("parameter {:?} is uncorrectable :/", &file_path);
        return Err(format!("parameter {:?} is uncorrectable :/", &file_path));
    }
    // my start1
    // my end1
    let mut line1 = 0;
    let mut line2 = 0;

    let colon_kind_mb = colon_lines_range_from_arg(&mut file_path);

    let gradient_type = gradient_type_from_range_kind(&colon_kind_mb);

    let cpath = crate::files_correction::canonical_path(&file_path);
    // my start2
// insert2
// insert2
// insert2
    // my end2
    if line1 == 0 && line2 == 0 {
        line2 = file_text.lines().count()
    }
    // my start3
// insert3
// insert3
// insert3
// insert3
    // my end3
    // my start4
//insert4
    // my end4
"""[1:-1]

payload = {
    "apply": [True, True, True, True, True],
    "chunks": [
        {
            "file_name": str(test_file),
            "file_action": "edit",
            "line1": 1,
            "line2": 4,
            "lines_remove": orig0,
            "lines_add": i0
        },
        {
            "file_name": str(test_file),
            "file_action": "edit",
            "line1": 9,
            "line2": 10,
            "lines_remove": orig1,
            "lines_add": i1
        },
        {
            "file_name": str(test_file),
            "file_action": "edit",
            "line1": 20,
            "line2": 25,
            "lines_remove": orig2,
            "lines_add": i2
        },
        {
            "file_name": str(test_file),
            "file_action": "edit",
            "line1": 30,
            "line2": 40,
            "lines_remove": orig3,
            "lines_add": i3
        },
        {
            "file_name": str(test_file),
            "file_action": "edit",
            "line1": 42,
            "line2": 43,
            "lines_remove": orig4,
            "lines_add": i4
        },
    ]
}


def diff_apply():
    url = "http://localhost:8001/v1/diff-apply"
    response = requests.post(url, data=json.dumps(payload))
    print(f"DIFF APPLY REQUEST: {response.status_code}: {response.text}")
    assert response.status_code == 200


def diff_undo():
    url = "http://localhost:8001/v1/diff-undo"
    response = requests.post(url, data=json.dumps(payload))
    print(f"DIFF UNDO REQUEST: {response.status_code}: {response.text}")
    assert response.status_code == 200


def diff_applied_chunks():
    url = "http://localhost:8001/v1/diff-applied-chunks"
    p = payload
    del p['apply']
    response = requests.post(url, data=json.dumps(p))
    print(f"DIFF APPLIED CHUNKS: {response.status_code}: {response.text}")
    assert response.status_code == 200


def test():
    # diff_applied_chunks()

    # with test_file.open("w") as f:
    #     f.write(file_text)

    # diff_apply()

    # assert text_after_apply == test_file.read_text()
    # print(colored("APPLY PASSED", "green"))
    #
    diff_undo()
    #
    # assert file_text == test_file.read_text()
    # print(colored("UNDO PASSED", "green"))


def main():
    test()


if __name__ == "__main__":
    main()