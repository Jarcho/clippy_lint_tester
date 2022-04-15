fn main() {
    println!("{}", foo());
}

fn foo() -> &'static str {
    return "Hello, world!";
}
