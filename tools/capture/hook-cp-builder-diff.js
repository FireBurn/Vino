"use strict";
// Diff a CP message across the builder call, hooking a REAL function entry.
//
//   6f1c80  push %rbp / mov %rsp,%rbp      <- the true entry
//   6f1c92  mov %rsi,%r12                  arg2 is the message buffer
//   6f1cf7  mov %r12,%rdx                  ... which becomes rdx at the dispatch
//   6f1d01  call *%r10                     vtable slot +0x10, ends in the seal
//
// Hooking mid-function addresses (the call at 0x6f1d01, its return site 0x6f1d04, the CTR loop at
// 0x1cf436, the AES core at 0x269dd0) segfaults DLM -- the trampoline lands on instructions that
// are branch targets. A function entry is safe and gives onLeave for free, which is what makes a
// before/after diff of the same buffer possible without any key.
function findModule(n){const m=Process.enumerateModules();for(let i=0;i<m.length;i++){if(m[i].name.indexOf(n)!==-1)return m[i];}return null;}
const dlm=findModule("DisplayLinkManager");
const N=32;
let n=0;
function rd(p,k){try{return Array.from(new Uint8Array(ptr(p).readByteArray(k)));}catch(e){return null;}}
Interceptor.attach(dlm.base.add(0x6f1c80),{
  onEnter:function(args){ this.addr=args[1].toString(); this.pre=rd(args[1],N); },
  onLeave:function(){
    if(!this.pre) return;
    const post=rd(this.addr,N);
    if(!post) return;
    n+=1; if(n>40) return;
    const diff=[]; for(let i=0;i<N;i++) if(this.pre[i]!==post[i]) diff.push(i);
    send({k:"ba",n:n,addr:this.addr,r8:"",rcx:"",r9:"",pre:this.pre,post:post,diff:diff});
  }
});
send({k:"ready"});
