import json
import asyncio
import traceback

import termcolor

from argparse import ArgumentParser

from swe.utils import AgentRunner
from swe.utils import get_swe_bench_lite_instance
from swe.steps import ProducePatchStep
from swe.utils.common import patched_file

from pathlib import Path
from typing import Dict, Any, Tuple


# MODEL = "gpt-4o"
# MODEL = "gpt-4o-2024-05-13"
# MODEL = "gpt-4o-2024-08-06"
MODEL = "gpt-4o-mini"


class SWERunner(AgentRunner):

    async def _steps(self, base_url: str, repo_path: Path, *args, **kwargs) -> Tuple[Dict[str, Any], str]:
        results: Dict[str, Any] = dict()
        problem_statement = kwargs["problem_statement"]
        filename = patched_file(kwargs["problem_patch"])
        step = ProducePatchStep(
            base_url=base_url, model_name=MODEL, patch_n=3, patch_choices=7, temperature=0.8)
        try:
            results["model_patches"], results["step2_results"] = await step.process(
                problem_statement=problem_statement,
                context_files=kwargs["found_files"],
                context_symbols=kwargs["found_symbols"],
                to_change_files=kwargs["to_change_files"],
                repo_path=repo_path)
            # NOTE: select top1 patch for eval without any postprocessing like in step3
            results["model_patch"] = "" if not results["model_patches"] else results["model_patches"][0][0]
            results["patched_file_in_model_patches"] = any([
                filename in model_patch
                for model_patch, _ in results["model_patches"]
            ])
        except Exception as e:
            results["error"] = f"step2: {type(e)} {str(e) or traceback.format_exc()}"
        results["model_name"] = step.model_name
        results["usage"] = step.usage
        return results, step.trajectory


async def main():
    parser = ArgumentParser()
    parser.add_argument("instance_id", type=str, help="SWE instance id")
    parser.add_argument("--timeout", type=float, default=None, help="processing timeout")
    parser.add_argument("--output-dir", type=Path, default=None, help="output directory")
    parser.add_argument("--step1-output", type=Path, default=None, help="step1 output filename")
    args = parser.parse_args()

    if args.output_dir is not None:
        args.output_dir.mkdir(exist_ok=True, parents=True)
        result_filename = args.output_dir / f"{args.instance_id}.json"
        traj_filename = args.output_dir / f"{args.instance_id}.md"
        if result_filename.exists():
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
    traj = ""

    try:
        if args.step1_output is not None:
            data = json.loads(args.step1_output.read_text())
            results["found_files"] = data["found_files"]
            results["found_symbols"] = data["found_symbols"]
            results["to_change_files"] = data["to_change_files"]
        else:
            results["found_files"] = [patched_file(results["problem_patch"])]
            results["found_symbols"] = []
            results["to_change_files"] = [patched_file(results["problem_patch"])]

        found_files_info = "\n".join([f"using additional step1 data:", *results["found_files"]])
        print(termcolor.colored(found_files_info, "green"))

        runner = SWERunner(
            timeout=args.timeout,
            use_ast=True,
            use_vecdb=False,
        )
        r, traj = await runner.run(
            repo_name=instance["repo"],
            base_commit=instance["base_commit"],
            **results,
        )
        results.update(**r, **results)
    except Exception as e:
        results["error"] = str(e) or traceback.format_exc()

    if args.output_dir is not None:
        with open(result_filename, "w") as f:
            json.dump(results, f, indent=4)
        with open(traj_filename, "w") as f:
            f.write(traj)
    else:
        print(json.dumps(results, indent=4))

    return results


if __name__ == "__main__":
    asyncio.run(main())
