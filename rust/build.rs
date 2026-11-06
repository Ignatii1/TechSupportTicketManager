fn main() {
    // стиль Fluent — как WPF-UI у C#-версии; на всех платформах один (на Linux — для проверок и снимков)
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/board.slint", config).expect("ui/board.slint собирается");
}
