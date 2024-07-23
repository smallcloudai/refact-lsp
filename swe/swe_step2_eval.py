import json
import asyncio
import os.path
import traceback

import jsonlines
import termcolor
import whatthepatch

from argparse import ArgumentParser

from agent_runner import AgentRunner
from agent_runner import get_swe_bench_lite_instance
from step2 import ProducePatchStep

from pathlib import Path
from typing import Dict, Any


# MODEL = "gpt-3.5-turbo"
MODEL = "gpt-4o"
# MODEL = "claude-3-5-sonnet"


def patched_file(patch: str) -> str:
    files = list(whatthepatch.parse_patch(patch))
    assert len(files) == 1
    header = files[0].header
    assert header.old_path[len("a/"):] == header.new_path[len("b/"):]
    return header.old_path[len("a/"):]


def full_path(instance_id, file_path, repo_path):
    if instance_id.startswith("django"):
        return repo_path / file_path
    elif instance_id.startswith("astropy"):
        return repo_path / file_path
    else:
        assert False


class SWERunner(AgentRunner):

    async def _steps(self, base_url: str, repo_path: Path, *args, **kwargs) -> Dict[str, Any]:
        results: Dict[str, Any] = dict()
        files = list(map(lambda x: str(full_path(kwargs['instance_id'], x, repo_path)), kwargs["step1_data"]))
        for file in files:
            assert os.path.exists(file)
        results["summarized_problem_statement"] = kwargs["problem_statement"]
        step = ProducePatchStep(base_url=base_url, model_name=MODEL, attempts=1, files=files)
        try:
            results["model_patches"] = \
                await step.process(task=results["summarized_problem_statement"], repo_path=repo_path)
        except Exception as e:
            raise RuntimeError(f"step2: {type(e)} {str(e) or traceback.format_exc()}")
        return results


async def main():
    parser = ArgumentParser()
    parser.add_argument("instance_id", type=str, help="SWE instance id")
    parser.add_argument("--timeout", type=float, default=None, help="processing timeout")
    parser.add_argument("--output-dir", type=Path, default=None, help="output directory")
    parser.add_argument("--step1-output", type=str, default=None, help="step1 output filename")
    args = parser.parse_args()

    if args.output_dir is not None:
        args.output_dir.mkdir(exist_ok=True, parents=True)
        output_filename = args.output_dir / f"{args.instance_id}.json"
        if output_filename.exists():
            print(f"skip {args.instance_id} because it's already done")
            exit(0)

    instance = get_swe_bench_lite_instance(args.instance_id)
    run_postfix = f"-{args.output_dir.name}" if args.output_dir is not None else ""
    results = {
        "model_name_or_path": f"refact-dev-{MODEL}{run_postfix}",
        "instance_id": args.instance_id,
        "problem_statement": instance["problem_statement"],
        "problem_patch": instance["patch"],
    }
    # agentless files
    data = list(jsonlines.open("/home/svakhreev/projects/refact-lsp/swe/loc_outputs.jsonl"))
    files = next((item["found_files"] for item in data if item["instance_id"] == args.instance_id))

    try:
        if isinstance(args.step1_output, str):
            data = json.loads(Path(args.step1_output).read_text())
            filenames_list = "\n".join(filter(
                lambda x: "test" not in x,
                data.get("summarized_problem_statement", "").split("\n")
            ))
            if filenames_list:
                results["step1_data"] = f"Use these files to solve the problem:\n{filenames_list}"
            else:
                results["step1_data"] = ""
        else:
            results["step1_data"] = files

        print(termcolor.colored(f"using additional step1 data:\n\n{results['step1_data']}", "green"))

        runner = SWERunner(
            timeout=args.timeout)
        results.update(await runner.run(
            repo_name=instance["repo"],
            base_commit=instance["base_commit"],
            **results,
        ))
    except Exception as e:
        results["error"] = f"{e}\n\n{traceback.format_exc()}"

    if args.output_dir is not None:
        with open(output_filename, "w") as f:
            json.dump(results, f, indent=4)
    else:
        print(json.dumps(results, indent=4))

    return results


if __name__ == "__main__":
    asyncio.run(main())
