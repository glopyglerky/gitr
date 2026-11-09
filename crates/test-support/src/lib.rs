//! Typed Rust constructors for hostile and valid Wasm test modules. No text interpreter.
use wasm_encoder::*;
#[derive(Clone, Copy, Debug)]
pub enum Guest { Empty, Echo, Fresh, Failure(i32), Trap, Spin, Grow, Buffer(i32,i32), Calls, Output, Wasi, Network, WrongImport, Start, WrongExport, Huge }
pub fn guest(kind: Guest) -> Vec<u8> {
    use Instruction::*;
    let mut m = Module::new();
    let mut t = TypeSection::new();
    t.ty().function([], [ValType::I32]); // run and len
    t.ty().function([ValType::I32,ValType::I32,ValType::I32], [ValType::I32]);
    t.ty().function([ValType::I32,ValType::I32], [ValType::I32]);
    t.ty().function([], []);
    t.ty().function([ValType::I64], []);
    t.ty().function([ValType::I32], [ValType::I32]);
    m.section(&t);
    let mut imports = ImportSection::new();
    let count = match kind {
        Guest::Echo => { for (name,ty) in [("input_len",0),("input_read",1),("output_write",2)] {imports.import("gitr_actions_v1",name,EntityType::Function(ty));} 3 },
        Guest::Fresh|Guest::Failure(_)|Guest::Trap|Guest::Buffer(..)|Guest::Output => {imports.import("gitr_actions_v1","output_write",EntityType::Function(2));1},
        Guest::Calls => {imports.import("gitr_actions_v1","input_len",EntityType::Function(0));1},
        Guest::Wasi => {imports.import("wasi_snapshot_preview1","fd_write",EntityType::Function(3));1},
        Guest::Network => {imports.import("gitr_actions_v1","network",EntityType::Function(3));1},
        Guest::WrongImport => {imports.import("gitr_actions_v1","input_len",EntityType::Function(4));1},
        _ => 0,
    };
    if count>0 {m.section(&imports);}
    let mut fs = FunctionSection::new();
    fs.function(if matches!(kind, Guest::WrongExport) {5} else {0});
    if matches!(kind, Guest::Start) {fs.function(3);}
    m.section(&fs);
    let mut mem = MemorySection::new();
    mem.memory(MemoryType {minimum: if matches!(kind,Guest::Huge) {257} else if matches!(kind,Guest::Echo|Guest::Output) {16} else {1}, maximum:None,memory64:false,shared:false,page_size_log2:None});
    m.section(&mem);
    if matches!(kind,Guest::Fresh) {let mut g=GlobalSection::new();g.global(GlobalType{val_type:ValType::I32,mutable:true,shared:false},&ConstExpr::i32_const(0));m.section(&g);}
    let mut ex=ExportSection::new();ex.export("memory",ExportKind::Memory,0);ex.export("run",ExportKind::Func,count);m.section(&ex);
    if matches!(kind,Guest::Start) {m.section(&StartSection{function_index:count+1});}
    let mut f=Function::new([]);
    let mut emit=|i: Instruction<'_>| {f.instruction(&i);};
    match kind {
        Guest::Echo => {for i in [I32Const(0),I32Const(0),Call(0),Call(1),Drop,I32Const(0),Call(0),Call(2),Drop] {emit(i);}},
        Guest::Fresh => {for i in [GlobalGet(0),I32Const(1),I32Add,GlobalSet(0),I32Const(0),GlobalGet(0),I32Store8(MemArg{offset:0,align:0,memory_index:0}),I32Const(0),I32Const(1),Call(0),Drop] {emit(i);}},
        Guest::Failure(_)|Guest::Trap => {for i in [I32Const(0),I32Const(7),Call(0),Drop] {emit(i);}},
        Guest::Spin => {for i in [Loop(BlockType::Empty),Br(0),End] {emit(i);}},
        Guest::Grow => {for i in [I32Const(10000),MemoryGrow(0),Drop] {emit(i);}},
        Guest::Buffer(p,l) => {for i in [I32Const(p),I32Const(l),Call(0),Drop] {emit(i);}},
        Guest::Calls => {for _ in 0..5000 {emit(Call(0));emit(Drop);}},
        Guest::Output => {for len in [1048576,1] {for i in [I32Const(0),I32Const(len),Call(0),Drop] {emit(i);}}},
        _ => {},
    }
    emit(if matches!(kind,Guest::Trap) {Unreachable} else {I32Const(if let Guest::Failure(code)=kind {code} else {0})});
    emit(End);
    let mut code=CodeSection::new();code.function(&f);
    if matches!(kind,Guest::Start) {let mut start=Function::new([]);start.instruction(&End);code.function(&start);}
    m.section(&code);
    if matches!(kind,Guest::Failure(_)|Guest::Trap) {let mut d=DataSection::new();d.active(0,&ConstExpr::i32_const(0),b"partial".iter().copied());m.section(&d);}
    m.finish()
}
