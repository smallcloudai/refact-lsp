from refact.chat_client import Message
from refact.chat_client import Usage
from refact.chat_client import FunctionDict
from refact.chat_client import ToolCallDict
from refact.chat_client import gen_function_call_id
from refact.chat_client import tools_fetch_and_filter
from refact.chat_client import ask_using_http

from typing import Set, Any, List, Iterable, Dict


__all__ = ["Step"]


class Step:
    def __init__(
            self,
            base_url: str,
            model_name: str,
            temperature: float = 0.2,
            max_depth: int = 8,
            *args, **kwargs
    ):
        self._base_url = base_url
        self._model_name = model_name
        self._temperature = temperature
        self._max_depth = max_depth
        self._usages = []
        self._trajectory = []

    @property
    def _tools(self) -> Set[str]:
        raise NotImplementedError()

    async def _query(
            self,
            messages: List[Message],
            stream: bool = False,
            only_deterministic_messages: bool = False) -> List[Message]:
        tools = await tools_fetch_and_filter(
            base_url=self._base_url,
            tools_turn_on=self._tools)
        assistant_choices = await ask_using_http(
            self._base_url, messages, 1, self._model_name,
            tools=tools, verbose=False, temperature=self._temperature,
            stream=stream, max_tokens=2048,
            only_deterministic_messages=only_deterministic_messages,
        )
        new_messages = assistant_choices[0][len(messages):]
        self._usages.extend([m.usage for m in new_messages])
        return new_messages

    async def _query_choices(self, messages: List[Message], n: int) -> List[List[Message]]:
        tools = await tools_fetch_and_filter(
            base_url=self._base_url,
            tools_turn_on=self._tools)
        assistant_choices = await ask_using_http(
            self._base_url, messages, n, self._model_name,
            tools=tools, verbose=False, temperature=self._temperature,
            stream=False, max_tokens=2048,
            only_deterministic_messages=False,
        )
        result = []
        for choice in assistant_choices:
            new_messages = choice[len(messages):]
            self._usages.extend([m.usage for m in new_messages])
            result.append(new_messages)
        return result

    async def _deterministic_tool_call_messages(
            self, functions: List[FunctionDict]) -> List[Message]:
        tool_calls = [
            ToolCallDict(id=gen_function_call_id(), function=function, type='function')
            for function in functions
        ]
        messages = [
            Message(role="assistant", finish_reason="tool_calls", tool_calls=tool_calls),
        ]
        tool_messages = await self._query(messages, only_deterministic_messages=True)
        return messages + tool_messages

    @property
    def model_name(self) -> str:
        return self._model_name

    @property
    def usage(self) -> Dict[str, int]:
        # TODO: probably we need return model and it's usage
        result = {
            'completion_tokens': 0,
            'prompt_tokens': 0,
            'total_tokens': 0,
        }
        for usage in filter(lambda x: isinstance(x, Usage), self._usages):
            result["completion_tokens"] += usage.completion_tokens
            result["prompt_tokens"] += usage.prompt_tokens
            result["total_tokens"] += usage.total_tokens
        return result

    @property
    def trajectory(self) -> str:
        return "\n\n".join(self._trajectory)

    async def process(self, **kwargs) -> Any:
        raise NotImplementedError()
