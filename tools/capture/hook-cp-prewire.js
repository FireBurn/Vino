"use strict";
// The record builder at 0x6f1c90 dispatches through a vtable at slot +0x10 with a 16-byte length
// in r8. That is one level ABOVE the cipher, so it is safe to hook: attaching to the CTR loop at
// 0x1cf436 or the AES core at 0x269dd0 segfaults DLM outright (confirmed, core dumped).
//
//   6f1cb5  call _Znwm(0x20)     a 32-byte typed object
//   6f1cd0  mov $0x10,%r8d       length 16
//   6f1cdf  lea -0x40(%rbp),%rsi rsi -> local holding that object
//   6f1cf3  mov 0x10(%rdx),%r10  vtable slot +0x10
//   6f1d01  call *%r10
function findModule(n){const m=Process.enumerateModules();for(let i=0;i<m.length;i++){if(m[i].name.indexOf(n)!==-1)return m[i];}return null;}
function findExport(n){return (typeof Module.findGlobalExportByName==="function")?Module.findGlobalExportByName(n):Module.findExportByName(null,n);}
const dlm=findModule("DisplayLinkManager");
const c={build:0,urb:0,sealed:0,plain:0};
function rd(p,n){try{return Array.from(new Uint8Array(ptr(p).readByteArray(n)));}catch(e){return null;}}
Interceptor.attach(dlm.base.add(0x6f1d01),{onEnter:function(){
  const x=this.context; c.build+=1;
  if(c.build<=8){
    send({k:"build",n:c.build,
      rdi:x.rdi.toString(), rdx:x.rdx.toString(), rcx:x.rcx.toString(),
      r8:x.r8.toString(), r9:x.r9.toString(),
      obj: rd(x.rsi,8),                 // the local holding the new object
      objDeref: (function(){try{return rd(ptr(x.rsi).readPointer(),32);}catch(e){return null;}})(),
      rdxMem: rd(x.rdx,32), rcxMem: rd(x.rcx,32), r9Mem: rd(x.r9,32)});
  }
}});
const SUBMITURB=0x8038550a;
Interceptor.attach(findExport("ioctl"),{onEnter:function(a){
  if((a[1].toInt32()>>>0)!==SUBMITURB) return;
  const u=a[2]; const len=u.add(24).readInt();
  if(len<=0||len>256) return;
  if(u.add(1).readU8()!==0x02) return;
  c.urb+=1;
  const b=new Uint8Array(u.add(16).readPointer().readByteArray(Math.min(len,16)));
  const sub=b[8]|(b[9]<<8);
  if(sub===0x24) c.sealed+=1; else if(sub===0x04) c.plain+=1;
}});
setInterval(function(){send({k:"c",c:c});},3000);
send({k:"ready"});
