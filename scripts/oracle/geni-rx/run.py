import subprocess,sys,json,shutil
from pathlib import Path
root=Path(sys.argv[1]);out=Path(sys.argv[2]);out.mkdir(exist_ok=False)
source=root/'src/devices/uart_qcom_geni.rs'
(out/'devices/uart_qcom_geni').mkdir(parents=True)
shutil.copyfile(source,out/'devices/uart_qcom_geni.rs')
for name in ['model.rs','tests.rs']:shutil.copyfile(source.parent/'uart_qcom_geni'/name,out/'devices/uart_qcom_geni'/name)
(out/'devices/mod.rs').write_text('pub mod uart_qcom_geni;\n')
harness=out/'harness.rs'
harness.write_text('mod sync {pub struct CleanLockToken;}\nmod scheme {pub mod debug {pub fn debug_input(_:u8,_:&mut crate::sync::CleanLockToken){} pub fn debug_notify(_:&mut crate::sync::CleanLockToken){}}}\nmod devices;\n')
b=subprocess.run(['rustc','--edition=2024','--test',str(harness),'-o',str(out/'tests')],capture_output=True,text=True)
(out/'compile.log').write_text(b.stdout+b.stderr)
t=subprocess.run([str(out/'tests'),'--nocapture'],capture_output=True,text=True) if b.returncode==0 else None
if t:(out/'tests.log').write_text(t.stdout+t.stderr)
r={'compile_exit':b.returncode,'test_exit':None if t is None else t.returncode};(out/'RESULT.json').write_text(json.dumps(r));print(json.dumps(r));sys.exit(b.returncode or (t.returncode if t else 1))
