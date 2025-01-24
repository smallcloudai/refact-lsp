-- init.lua
-- Ensure you have toggleterm installed
require('toggleterm').setup{}

local Terminal = require('toggleterm.terminal').Terminal

-- Create a terminal instance for refact
local refact = Terminal:new({ cmd = "refact .", direction = "float", hidden = true })

-- Function to toggle refact terminal
function _refact_toggle()
  refact:toggle()
end

-- Key mappings for toggling the refact terminal
vim.api.nvim_set_keymap("n", "<A-e>", "<cmd>lua _refact_toggle()<CR>", { noremap = true, silent = true })
vim.api.nvim_set_keymap("t", "<A-e>", "<cmd>lua _refact_toggle()<CR>", { noremap = true, silent = true })

-- Create a terminal instance for chat
local chat = Terminal:new({ cmd = "python3 chat_with_at_command.py", direction = "float", hidden = true })

-- Function to toggle chat terminal
function _chat_toggle()
  chat:toggle()
end

-- Key mapping for toggling the chat terminal
vim.api.nvim_set_keymap("n", "<A-c>", "<cmd>lua _chat_toggle()<CR>", { noremap = true, silent = true })
vim.api.nvim_set_keymap("t", "<A-c>", "<cmd>lua _chat_toggle()<CR>", { noremap = true, silent = true })
