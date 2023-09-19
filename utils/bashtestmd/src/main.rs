use std::collections::VecDeque;
use std::io::{self, Write};

use clap::Parser;
use markdown::mdast;

#[derive(Debug, Parser)]
struct Args {
    /// Input Markdown file to parse
    #[clap(short, long)]
    input: String,
    /// Path to output Bash script
    #[clap(short, long)]
    output: String,
    /// Only run code blocks with this tag
    #[clap(short, long)]
    tag: String,
}

fn main() {
    let args = Args::parse();

    let file_contents = std::fs::read_to_string(&args.input).unwrap();
    let markdown_parse_options = markdown::ParseOptions::gfm();
    let markdown_ast = markdown::to_mdast(&file_contents, &markdown_parse_options).unwrap();

    let code_blocks = get_all_code_blocks(markdown_ast);
    let commands = convert_code_blocks_into_commands(code_blocks, &args.tag);
    let script = compile_commands_into_bash(commands);

    std::fs::write(&args.output, script).unwrap();
}

struct Command {
    cmd: String,
    long_running: bool,
    expected_output: Option<String>,
    exit_code: Option<i32>,
}

impl Command {
    fn new(cmd: &str) -> Self {
        Self {
            cmd: cmd.to_string(),
            long_running: false,
            expected_output: None,
            exit_code: Some(0),
        }
    }

    fn compile(&self, mut w: impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "echo {}",
            shell_escape::escape(format!("Running: '{}'", self.cmd).into())
        )?;

        if self.long_running {
            writeln!(w, "{} &", self.cmd)?;
            writeln!(w, "sleep 20")?;
            return Ok(());
        }

        if let Some(output) = &self.expected_output {
            writeln!(
                w,
                r#"
output=$({})
expected={}
# Either of the two must be a substring of the other. This kinda protects us
# against whitespace differences, trimming, etc.
if ! [[ $output == *"$expected"* || $expected == *"$output"* ]]; then
    echo "'$expected' not found in text:"
    echo "'$output'"
    exit 1
fi
"#,
                self.cmd,
                shell_escape::escape(output.into())
            )?;
        } else {
            writeln!(w, "{}", self.cmd)?;
        }

        if let Some(exit_code) = self.exit_code {
            writeln!(w, "if [ $? -ne {} ]; then", exit_code)?;
            writeln!(w, "    echo \"Expected exit code {}, got $?\"", exit_code)?;
            writeln!(w, "    exit 1")?;
            writeln!(w, "fi")?;
        }

        Ok(())
    }
}

fn compile_commands_into_bash(cmds: Vec<Command>) -> String {
    let mut script = Vec::<u8>::new();
    // Shebang.
    writeln!(&mut script, "#!/usr/bin/env bash").unwrap();
    writeln!(&mut script, r#"trap 'jobs -p | xargs -r kill' EXIT"#).unwrap();

    for cmd in cmds {
        cmd.compile(&mut script).unwrap();
    }
    writeln!(&mut script, r#"echo "All tests passed!"; exit 0"#).unwrap();
    String::from_utf8(script).unwrap()
}

struct CodeBlockTags {
    long_running: bool,
    compare_output: bool,
    exit_code: Option<i32>,
}

impl CodeBlockTags {
    fn parse(code_block: &mdast::Code) -> Self {
        let langs: Vec<String> = code_block
            .lang
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::to_string)
            .collect();

        let mut tags = Self {
            long_running: false,
            compare_output: false,
            exit_code: Some(0),
        };

        for lang in langs {
            if lang == "bashtestmd:long-running" {
                tags.long_running = true;
            } else if lang == "bashtestmd:compare-output" {
                tags.compare_output = true;
            } else if lang == "bashtestmd:exit-code-ignore" {
                tags.exit_code = None;
            } else if lang.starts_with("bashtestmd:exit-code=") {
                let exit_code = lang.split_once('=').unwrap().1.parse().unwrap();
                tags.exit_code = Some(exit_code);
            }
        }

        tags
    }
}

fn convert_code_blocks_into_commands(
    code_blocks: Vec<mdast::Code>,
    only_tag: &str,
) -> Vec<Command> {
    const PROMPT: &str = "$ ";

    let mut commands = Vec::new();

    for code_block in code_blocks {
        if !code_block
            .lang
            .as_deref()
            .unwrap_or_default()
            .contains(only_tag)
        {
            continue;
        }
        let tags = CodeBlockTags::parse(&code_block);

        let mut cmd: Option<String> = None;
        let mut output = String::new();

        for line in code_block.value.lines() {
            if let Some(cmd_string) = line.strip_prefix(PROMPT) {
                if let Some(cmd) = cmd {
                    commands.push(Command::new(&cmd));
                }
                cmd = Some(cmd_string.to_string());
            } else {
                output.push_str(line);
                output.push('\n');
            }
        }
        if let Some(cmd) = cmd {
            let mut cmd = Command::new(&cmd);
            cmd.long_running = tags.long_running;
            cmd.expected_output = if tags.compare_output {
                Some(output)
            } else {
                None
            };
            commands.push(cmd);
        }
    }

    commands
}

/// Ordered list of all code blocks in the Markdown file.
fn get_all_code_blocks(markdown_ast: mdast::Node) -> Vec<mdast::Code> {
    let mut code_blocks = Vec::new();

    let mut nodes: VecDeque<mdast::Node> = markdown_ast
        .children()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();

    while let Some(next_node) = nodes.pop_front() {
        if let mdast::Node::Code(code_node) = next_node {
            code_blocks.push(code_node);
        } else {
            let children = next_node.children().map(Vec::as_slice).unwrap_or_default();
            for child in children.iter() {
                nodes.push_front(child.clone());
            }
        }
    }

    code_blocks
}
