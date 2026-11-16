//! The Formula node's language: arithmetic on up to three inputs (`a`, `b`,
//! `c`) with a fixed set of functions. It has no variables, loops, strings
//! or calls out, so a formula can only compute a number. Formulas are parsed
//! once into a short stack program; evaluating one per dab allocates nothing.

/// Longest formula, in characters and in tokens.
pub const MAX_CHARS: usize = 200;
const MAX_TOKENS: usize = 64;
/// Deepest the evaluation stack can get.
const MAX_STACK: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Func {
    Min,
    Max,
    Clamp,
    Abs,
    Floor,
    Fract,
    Sqrt,
    Pow,
    Sin,
    Cos,
    Mix,
    Smoothstep,
    Step,
}
impl Func {
    fn named(name: &str) -> Option<(Func, usize)> {
        Some(match name {
            "min" => (Self::Min, 2),
            "max" => (Self::Max, 2),
            "clamp" => (Self::Clamp, 3),
            "abs" => (Self::Abs, 1),
            "floor" => (Self::Floor, 1),
            "fract" => (Self::Fract, 1),
            "sqrt" => (Self::Sqrt, 1),
            "pow" => (Self::Pow, 2),
            "sin" => (Self::Sin, 1),
            "cos" => (Self::Cos, 1),
            "mix" => (Self::Mix, 3),
            "smoothstep" => (Self::Smoothstep, 3),
            "step" => (Self::Step, 2),
            _ => return None,
        })
    }
}

/// The names a formula can use, for help text.
pub const NAMES: &str =
    "a, b, c, pi; min, max, clamp, abs, floor, fract, sqrt, pow, sin, cos, mix, smoothstep, step";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Number(f32),
    Input(u8),
    Negate,
    Add,
    Subtract,
    Multiply,
    Divide,
    Call(Func),
}

/// A parsed formula, ready to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f32),
    Name(String),
    Symbol(char),
}

fn tokens(source: &str) -> Result<Vec<Token>, String> {
    if source.chars().count() > MAX_CHARS {
        return Err(format!("Formulas are at most {MAX_CHARS} characters"));
    }
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let value: f32 = text
                .parse()
                .map_err(|_| format!("“{text}” is not a number"))?;
            out.push(Token::Number(value));
        } else if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Token::Name(chars[start..i].iter().collect()));
        } else if "+-*/(),".contains(c) {
            out.push(Token::Symbol(c));
            i += 1;
        } else {
            return Err(format!("“{c}” cannot be used in a formula"));
        }
        if out.len() > MAX_TOKENS {
            return Err(format!("Formulas are at most {MAX_TOKENS} parts long"));
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    steps: Vec<Step>,
}
impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }
    fn eat(&mut self, symbol: char) -> bool {
        if self.peek() == Some(&Token::Symbol(symbol)) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expression(&mut self) -> Result<(), String> {
        self.term()?;
        loop {
            if self.eat('+') {
                self.term()?;
                self.steps.push(Step::Add);
            } else if self.eat('-') {
                self.term()?;
                self.steps.push(Step::Subtract);
            } else {
                return Ok(());
            }
        }
    }
    fn term(&mut self) -> Result<(), String> {
        self.unary()?;
        loop {
            if self.eat('*') {
                self.unary()?;
                self.steps.push(Step::Multiply);
            } else if self.eat('/') {
                self.unary()?;
                self.steps.push(Step::Divide);
            } else {
                return Ok(());
            }
        }
    }
    fn unary(&mut self) -> Result<(), String> {
        if self.eat('-') {
            self.unary()?;
            self.steps.push(Step::Negate);
            Ok(())
        } else {
            self.eat('+');
            self.primary()
        }
    }
    fn primary(&mut self) -> Result<(), String> {
        match self.tokens.get(self.at).cloned() {
            Some(Token::Number(value)) => {
                self.at += 1;
                self.steps.push(Step::Number(value));
                Ok(())
            }
            Some(Token::Symbol('(')) => {
                self.at += 1;
                self.expression()?;
                if self.eat(')') {
                    Ok(())
                } else {
                    Err("A “(” is missing its “)”".into())
                }
            }
            Some(Token::Name(name)) => {
                self.at += 1;
                match name.as_str() {
                    "a" | "b" | "c" => {
                        self.steps.push(Step::Input(name.as_bytes()[0] - b'a'));
                        return Ok(());
                    }
                    "pi" => {
                        self.steps.push(Step::Number(std::f32::consts::PI));
                        return Ok(());
                    }
                    _ => {}
                }
                let (func, arity) =
                    Func::named(&name).ok_or_else(|| format!("“{name}” is not a known name"))?;
                if !self.eat('(') {
                    return Err(format!("{name} needs its values in brackets: {name}(…)"));
                }
                for i in 0..arity {
                    if i > 0 && !self.eat(',') {
                        return Err(format!("{name} needs {arity} values"));
                    }
                    self.expression()?;
                }
                if !self.eat(')') {
                    return Err(format!("{name} takes {arity} values"));
                }
                self.steps.push(Step::Call(func));
                Ok(())
            }
            Some(Token::Symbol(c)) => Err(format!("“{c}” is out of place")),
            None => Err("The formula ends too soon".into()),
        }
    }
}

impl Program {
    pub fn parse(source: &str) -> Result<Self, String> {
        let tokens = tokens(source)?;
        if tokens.is_empty() {
            return Err("Write a formula, such as a * b".into());
        }
        let mut parser = Parser {
            tokens,
            at: 0,
            steps: Vec::new(),
        };
        parser.expression()?;
        if let Some(token) = parser.peek() {
            return Err(match token {
                Token::Symbol(c) => format!("“{c}” is out of place"),
                _ => "Two values follow each other without an operator".into(),
            });
        }
        let program = Program {
            steps: parser.steps,
        };
        if program.depth() > MAX_STACK {
            return Err("The formula is nested too deeply".into());
        }
        Ok(program)
    }
    /// Which of a, b and c the formula reads.
    pub fn reads(&self) -> [bool; 3] {
        let mut reads = [false; 3];
        for step in &self.steps {
            if let Step::Input(i) = step {
                reads[usize::from(*i)] = true;
            }
        }
        reads
    }
    fn depth(&self) -> usize {
        let (mut depth, mut deepest) = (0usize, 0usize);
        for step in &self.steps {
            match step {
                Step::Number(_) | Step::Input(_) => depth += 1,
                Step::Negate => {}
                Step::Add | Step::Subtract | Step::Multiply | Step::Divide => depth -= 1,
                Step::Call(func) => {
                    let (_, arity) = Func::named(func_name(*func)).unwrap();
                    depth -= arity - 1;
                }
            }
            deepest = deepest.max(depth);
        }
        deepest
    }
    /// Run on inputs `a`, `b`, `c`. A result that is not a finite number is
    /// missing (NaN), like an absent sensor.
    pub fn run(&self, inputs: [f32; 3]) -> f32 {
        let mut stack = [0.0f32; MAX_STACK];
        let mut top = 0usize;
        for step in &self.steps {
            match *step {
                Step::Number(v) => {
                    stack[top] = v;
                    top += 1;
                }
                Step::Input(i) => {
                    stack[top] = inputs[usize::from(i)];
                    top += 1;
                }
                Step::Negate => stack[top - 1] = -stack[top - 1],
                Step::Add | Step::Subtract | Step::Multiply | Step::Divide => {
                    top -= 1;
                    let (x, y) = (stack[top - 1], stack[top]);
                    stack[top - 1] = match step {
                        Step::Add => x + y,
                        Step::Subtract => x - y,
                        Step::Multiply => x * y,
                        _ => x / y,
                    };
                }
                Step::Call(func) => {
                    let (_, arity) = Func::named(func_name(func)).unwrap();
                    top -= arity - 1;
                    let args = &stack[top - 1..top - 1 + arity];
                    let x = args[0];
                    stack[top - 1] = match func {
                        Func::Min => x.min(args[1]),
                        Func::Max => x.max(args[1]),
                        Func::Clamp => x.max(args[1]).min(args[2]),
                        Func::Abs => x.abs(),
                        Func::Floor => x.floor(),
                        Func::Fract => x - x.floor(),
                        Func::Sqrt => x.sqrt(),
                        Func::Pow => x.powf(args[1]),
                        Func::Sin => x.sin(),
                        Func::Cos => x.cos(),
                        Func::Mix => x + (args[1] - x) * args[2],
                        Func::Smoothstep => {
                            let t = ((args[2] - x) / (args[1] - x)).clamp(0.0, 1.0);
                            t * t * (3.0 - 2.0 * t)
                        }
                        Func::Step => {
                            if args[1] >= x {
                                1.0
                            } else {
                                0.0
                            }
                        }
                    };
                }
            }
        }
        let result = stack[0];
        if result.is_finite() { result } else { f32::NAN }
    }
}

fn func_name(func: Func) -> &'static str {
    match func {
        Func::Min => "min",
        Func::Max => "max",
        Func::Clamp => "clamp",
        Func::Abs => "abs",
        Func::Floor => "floor",
        Func::Fract => "fract",
        Func::Sqrt => "sqrt",
        Func::Pow => "pow",
        Func::Sin => "sin",
        Func::Cos => "cos",
        Func::Mix => "mix",
        Func::Smoothstep => "smoothstep",
        Func::Step => "step",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(source: &str, inputs: [f32; 3]) -> f32 {
        Program::parse(source).unwrap().run(inputs)
    }

    #[test]
    fn formulas_follow_arithmetic_rules_and_know_their_functions() {
        let i = [0.5, 0.25, 2.0];
        assert_eq!(eval("1 + 2 * 3", i), 7.0);
        assert_eq!(eval("(1 + 2) * 3", i), 9.0);
        assert_eq!(eval("-a + b", i), -0.25);
        assert_eq!(eval("a - b - 0.125", i), 0.125);
        assert_eq!(eval("c / 4 / 2", i), 0.25);
        assert_eq!(eval("clamp(a * c + 1, 0, 1)", i), 1.0);
        assert_eq!(eval("mix(0, 10, a)", i), 5.0);
        assert_eq!(eval("smoothstep(0, 1, 0.5)", i), 0.5);
        assert_eq!(eval("step(0.5, a)", i), 1.0);
        assert_eq!(eval("step(0.6, a)", i), 0.0);
        assert_eq!(eval("min(a, b) + max(a, b)", i), 0.75);
        assert_eq!(eval("abs(-a) + floor(c + 0.5) + fract(1.25)", i), 2.75);
        assert_eq!(eval("pow(c, 3) + sqrt(4)", i), 10.0);
        assert!((eval("sin(pi / 2) + cos(0)", i) - 2.0).abs() < 1e-6);
        assert_eq!(
            Program::parse("a * a + c").unwrap().reads(),
            [true, false, true]
        );
        // Division by zero and other non-numbers are missing values.
        assert!(eval("a / 0", i).is_nan());
        assert!(eval("sqrt(-1)", i).is_nan());
    }

    #[test]
    fn mistakes_are_explained_and_nothing_else_is_reachable() {
        let refused = |source: &str, words: &str| {
            let error = Program::parse(source).unwrap_err();
            assert!(error.contains(words), "{source}: {error}");
        };
        refused("", "Write a formula");
        refused("a +", "ends too soon");
        refused("(a + b", "missing its");
        refused("a b", "without an operator");
        refused("x * 2", "not a known name");
        refused("min(a)", "needs 2 values");
        refused("min(a, b, c)", "takes 2 values");
        refused("sin a", "in brackets");
        refused("a ; b", "cannot be used");
        refused("a = 1", "cannot be used");
        refused("1..2", "not a number");
        refused(&"a+".repeat(40), "parts long");
        refused(&"a".repeat(201), "200 characters");
        refused(
            &format!("{}a{}", "(".repeat(40), ")".repeat(40)),
            "parts long",
        );
        let deep = format!("{}1{}", "min(1, ".repeat(31), ")".repeat(31));
        assert!(Program::parse(&deep).is_err());
    }
}
