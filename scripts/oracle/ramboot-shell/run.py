"""ONE fresh disposable single-CPU no-disk interactive emulator acceptance."""
import sys,datetime,zlib,hashlib,json,re,shutil,socket,struct,subprocess,time,traceback,os
from pathlib import Path
project=Path(__file__).resolve().parents[3]
root=Path(os.environ.get('R1_OUT_DIR', project/'out/emu-op6'/('r1-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S'))))
stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S')
out=root;out.mkdir(parents=True,exist_ok=False)
unit='op6-r1-'+stamp.lower();nonce='R1_'+stamp
scenario=os.environ.get('R1_SCENARIO','reboot')
result={'purpose':'R-1 emulator reset proof','unit':unit,'nonce':nonce,'scenario':scenario,'tests':{},'physical_commands':0,'scope':'singleCPU OnePlus emulator, 8GiB sparse, noUSB/no disk/no network, hostMemoryMax2GiB/Swap0'}
package=Path(sys.argv[1]);expected_sha=sys.argv[2]
raw=package/'u-boot-nodtb.bin'
image=package/'op6-embedded-efi-redox.img'
blob=image.read_bytes();assert hashlib.sha256(blob).hexdigest()==expected_sha
klen=struct.unpack_from('<I',blob,8)[0]
dec=zlib.decompressobj(16+zlib.MAX_WBITS);assert dec.decompress(blob[4096:4096+klen])==raw.read_bytes()
assert dec.eof and dec.unused_data==(project/'scripts/phone/embedded-efi/sdm845-oneplus-enchilada.dtb').read_bytes()
dtb=out/'interactive-oneplus6.dtb';shutil.copyfile(project/'scripts/phone/embedded-efi/sdm845-oneplus-enchilada.dtb',dtb)
def edit(*args):subprocess.run(['fdtput',*args],check=True,capture_output=True)
edit('-t','x',str(dtb),'/memory@80000000','reg','0','80000000','2','0')
for n in ['/soc@0/usb@a6f8800','/soc@0/usb@a8f8800']:edit('-t','s',str(dtb),n,'status','disabled')
edit('-t','s',str(dtb),'/chosen','redox,acceptance-nonce',nonce)
fresh=out/'interactive-uboot-fresh-dtb.bin';fresh.write_bytes(raw.read_bytes()+dtb.read_bytes())
result.update(raw_sha256=hashlib.sha256(fresh.read_bytes()).hexdigest(),dtb_sha256=hashlib.sha256(dtb.read_bytes()).hexdigest(),input_image_sha256=hashlib.sha256(blob).hexdigest())
(out/'ADMISSION.json').write_text(json.dumps(result,indent=2)+'\n')
# Unix paths must fit sockaddr_un; the socket is scoped to this unique unit.
sockpath=Path('/tmp')/(unit+'.sock');assert not sockpath.exists()
cmd=['systemd-run','--user','--unit',unit,'--wait','--pipe','-p','MemoryMax=2G','-p','MemorySwapMax=0','-p','RuntimeMaxSec=130','timeout','120','$HOME/Developer/forks/qemu-oneplus6/build/qemu-system-aarch64','-M','oneplus6','-m','8G','-smp','1','-display','none','-monitor','none','-chardev',f'socket,id=serial0,path={sockpath},server=on,wait=off','-serial','chardev:serial0','-net','none','-no-reboot','-device',f'loader,file={fresh},addr=0x80008000,cpu-num=0']
(out/'COMMAND.json').write_text(json.dumps(cmd,indent=2)+'\n')
log=(out/'driver.log').open('wb');proc=None;s=None;data=bytearray();started=time.monotonic()
def read_until(predicate,seconds):
 end=min(started+115,time.monotonic()+seconds)
 while time.monotonic()<end:
  if predicate(bytes(data)):return
  try:
   x=s.recv(65536)
   if not x:raise RuntimeError('serial EOF')
   data.extend(x);(out/'serial.log').write_bytes(data)
  except socket.timeout:pass
 raise TimeoutError('expected serial output absent before bound')
try:
 proc=subprocess.Popen(cmd,stdout=log,stderr=subprocess.STDOUT)
 end=time.monotonic()+12
 while not sockpath.exists() and time.monotonic()<end:time.sleep(.1)
 s=socket.socket(socket.AF_UNIX);s.settimeout(.25);s.connect(str(sockpath))
 read_until(lambda b:b'ram:~$ ' in b,75);result['tests']['shell_prompt']=True
 if scenario=='reboot':
  offset=len(data);s.sendall(b'reboot\n')
  read_until(lambda b:b'RAMBOOT: reset via PSCI in 0s' in b[offset:],8)
  result['tests']['reboot_reset_announcement']={'pass':True,'start':offset,'end':len(data)}
 elif scenario!='ttl':raise RuntimeError('R1_SCENARIO must be reboot or ttl')
 if scenario=='ttl':
  read_until(lambda b:b'RAMBOOT: reset via PSCI in ' in b,20)
  result['tests']['ramboot_reset_line']=True
 read_until(lambda b:b'kreset: PSCI SYSTEM_RESET (0x84000009)' in b or b'emergency_reset: PSCI SYSTEM_RESET (0x84000009)' in b,8 if scenario=='reboot' else 90)
 result['tests']['psci_system_reset_marker']=True
 try:
  while proc.poll() is None:
   try:
    x=s.recv(65536)
    if not x:break
    data.extend(x);(out/'serial.log').write_bytes(data)
   except socket.timeout:pass
  try:proc.wait(timeout=15)
  except subprocess.TimeoutExpired:pass
  result['qemu_exited_after_reset']=proc.poll() is not None
  if not result['qemu_exited_after_reset']:raise RuntimeError('QEMU did not exit after PSCI reset marker')
 except Exception:raise
 result['pass']=True
except Exception as e:
 result.update(pass_=False,error=str(e),traceback=traceback.format_exc());result['pass']=False
finally:
 if s:s.close()
 subprocess.run(['systemctl','--user','stop',unit],capture_output=True,timeout=10)
 if proc:
  try:result['driver_exit']=proc.wait(timeout=10)
  except subprocess.TimeoutExpired:result['driver_exit']='cleanup timeout'
 log.close()
 state=subprocess.run(['systemctl','--user','show',unit,'-p','ActiveState','-p','SubState','-p','MainPID','-p','MemoryPeak','-p','MemorySwapPeak'],capture_output=True,text=True,timeout=5)
 result['terminal_unit_state']=state.stdout;result['elapsed']=time.monotonic()-started;result['serial_sha256']=hashlib.sha256(data).hexdigest()
 (out/'serial.log').write_bytes(data);(out/'RESULT.json').write_text(json.dumps(result,indent=2)+'\n')
 if sockpath.exists():sockpath.unlink()
 print(json.dumps({'out':str(out),**result}),flush=True)
raise SystemExit(0 if result['pass'] else 1)
