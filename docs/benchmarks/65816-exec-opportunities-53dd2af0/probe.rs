use actionc::{compiler::native65816, includes::ModuleLoadOptions, mir65816::{self,Mir65816Op as Op}};
use serde_json::json;
use std::{path::PathBuf,fs};
fn kind(op: &Op)->String {
 match op {
 Op::Load {width,address,..}=>format!("Load/{}/{:?}",width.get(),address.mode),
 Op::Store {width,address,..}=>format!("Store/{}/{:?}",width.get(),address.mode),
 Op::AddressOf {width,..}=>format!("AddressOf/{}",width.get()),
 Op::Copy {bytes,..}=>format!("Copy/{}",bytes.get()),
 Op::Unary {width,operation,..}=>format!("Unary/{}/{operation:?}",width.get()),
 Op::Cast {from,to,kind,..}=>format!("Cast/{}->{}/{kind:?}",from.get(),to.get()),
 Op::PointerOffset {width,..}=>format!("PointerOffset/{}",width.get()),
 Op::Binary {width,operation,..}=>format!("Binary/{}/{operation:?}",width.get()),
 Op::Compare {width,signed,operation,..}=>format!("Compare/{}/{signed}/{operation:?}",width.get()),
 Op::Call {target,..}=>format!("Call/{}",match target {mir65816::Mir65816CallTarget::Indirect(..)=>"Indirect",_=>"Direct"}),
 }
}
fn main() {
 let args:Vec<_>=std::env::args().collect();
 let dir=PathBuf::from(&args[1]);let root=PathBuf::from(&args[2]);let out=PathBuf::from(&args[3]);let opt=args[4]=="true";
 let mut options:serde_json::Value=serde_json::from_slice(&fs::read(dir.join("layout.json")).unwrap()).unwrap();
 assert!(matches!(options.as_object_mut().unwrap().remove("stack_checks"), None | Some(serde_json::Value::Bool(true))));
 fs::write(out.with_extension("layout.json"),serde_json::to_vec_pretty(&options).unwrap()).unwrap();
 let layout:mir65816::image::LinkOptions=serde_json::from_value(options).unwrap();
 let mut modules=ModuleLoadOptions::default();modules.module_paths=vec![dir.join("task-kernel"),dir.clone(),root.join("examples"),root.join("lib")];
 let p=native65816::prepare_file(dir.join("kernel-program.act"),opt,&modules).unwrap();
 let c=p.compile(&layout).unwrap();fs::write(out.with_extension("image.json"),c.image.to_json().unwrap()).unwrap();
 let mut routines=vec![];
 for m in &c.machine.routines {
  let r=c.machine.prepared.routines.iter().find(|r|r.id==m.id).unwrap();
  let mut spans=vec![];
  for (&(id,index),range) in &m.code.mir_spans {
   let b=r.blocks.iter().find(|b|b.id==id).unwrap();
   let (kind,description)=if let Some(op)=b.ops.get(index){(kind(op),format!("{op:?}"))}else{(format!("Terminator/{}",format!("{:?}",b.terminator).split([' ','(']).next().unwrap()),format!("{:?}",b.terminator))};
   spans.push(json!({"block":id.0,"index":index,"start":range.start,"end":range.end,"kind":kind,"description":description}));
  }
  routines.push(json!({"id":r.id.0,"name":r.name,"entry":format!("{:?}",r.entry),"spans":spans,
    "parameters":r.frame.parameters.iter().map(|p|json!({"id":p.param.0,"incoming":format!("{:?}",p.incoming),"frame_object":p.frame_object.map(|id|id.0)})).collect::<Vec<_>>(),
    "objects":r.frame.objects.iter().map(|o|json!({"id":o.id.0,"owner":format!("{:?}",o.owner),"mutable":o.mutable,"addressable":o.addressable,"size":o.size.get()})).collect::<Vec<_>>(),
    "temps":r.temps.iter().map(|(id,ty)|json!({"id":id.0,"type":format!("{:?}",ty)})).collect::<Vec<_>>(),
    "blocks":r.blocks.iter().map(|b|json!({"id":b.id.0,"params":b.params.iter().map(|(id,w)|json!([id.0,w.get()])).collect::<Vec<_>>(),"ops":b.ops.iter().map(|op|format!("{:?}",op)).collect::<Vec<_>>(),"terminator":format!("{:?}",b.terminator)})).collect::<Vec<_>>(),
    "fixups":m.code.fixups.iter().map(|f|json!({"offset":f.offset,"target":format!("{:?}",f.target),"byte":f.byte,"addend":f.addend})).collect::<Vec<_>>(),
    "labels":m.code.labels.iter().map(|(l,at)|json!([l.0,at])).collect::<Vec<_>>(),
    "jumps":m.code.local_jumps.iter().map(|s|json!({"offset":s.offset,"target":s.target.0,"size":s.encoding.size()})).collect::<Vec<_>>(),
    "returns":m.code.return_fixups.iter().map(|(at,l)|json!([at,l.0])).collect::<Vec<_>>(),
    "dispatches":m.code.conditional_branches.iter().map(|b|json!({"offset":b.offset,"predicate":b.predicate,"short":b.short,"dispatch":b.dispatch,"target":b.target.0})).collect::<Vec<_>>()
  }));
 }
 let relocations=mir65816::relocation::collect(&c.machine.prepared,&c.machine).unwrap().into_iter().map(|f|json!({"owner":format!("{:?}",f.owner),"target":format!("{:?}",f.target),"offset":f.offset,"width":f.width,"selector":f.selector})).collect::<Vec<_>>();
 fs::write(out.with_extension("inventory.json"),serde_json::to_vec(&json!({"source_paths":p.source_paths,"routines":routines,"relocations":relocations})).unwrap()).unwrap();
 println!("{}: {} routines, {} code bytes",out.display(),routines.len(),c.image.routines.iter().map(|r|r.size).sum::<u32>());
}
