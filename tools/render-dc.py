"""Reconstruct a DLM frame from DC coefficients alone, using the Ella grammar.

The main section (16 significance units + 48 DC escapes) decodes at 100%, so a DC-only render is
a direct test of that grammar: get it right and a desktop appears, get it wrong and it is noise.
"""
import pickle, struct, sys
from PIL import Image

S = pickle.load(open('/tmp/claude-1000/-home-fireburn-Downloads-dl-scripts/18c08132-507c-43d4-991f-4666efa5de58/scratchpad/ella-strips.pkl','rb'))

def bits(bs):
    o=[]
    for b in bs:
        for i in range(8): o.append((b>>i)&1)
    return o
class R:
    def __init__(s,b): s.b=b; s.p=0
    def bit(s):
        if s.p>=len(s.b): raise EOFError
        v=s.b[s.p]; s.p+=1; return v
def esc_i(r,cmax):
    c=0; pay=[]
    while c<cmax:
        if not r.bit(): break
        pay.append(r.bit()); c+=1
    if c==0: return 0
    off=0
    for b in pay[:c-1]: off=(off<<1)|b
    return ((1<<(c-1))+off)*(1 if pay[c-1] else -1)
def n_chroma(r):
    c=1; v=r.bit()
    while r.bit(): v=(v<<1)|r.bit(); c+=1
    return ((1<<c)-1)+v
def n_luma(r):
    ones=0; v=0
    while ones<6:
        if not r.bit(): return (64-(1<<ones))-v
        v=(v<<1)|r.bit(); ones+=1
    r.bit(); return 0
def unit(r):
    if r.bit():
        lcr=n_chroma(r)
        if r.bit(): return lcr, n_chroma(r), n_luma(r)
        return lcr, 0, n_luma(r)
    if r.bit(): return 0, n_chroma(r), n_luma(r)
    return 0, 0, n_luma(r)

def strip_dc(s):
    """-> (x, y, [(cr,cb,y)] * 16) from the main section only."""
    r = R(bits(s[16:]))
    for _ in range(16): unit(r)
    out=[]; pcr=pcb=py=0
    for _ in range(16):
        pcr += esc_i(r,10); pcb += esc_i(r,10); py += esc_i(r,10)
        out.append((pcr,pcb,py))
    x,y = struct.unpack('<HH', s[2:6])
    return x, y, out

# One frame: strips arrive in transmission order and cover the surface once. Take strips until a
# coordinate repeats, which is where the next frame begins.
W,H = 1920,1088
BW,BH = W//8, H//8
img = [[0]*BW for _ in range(BH)]
chroma = [[(0,0)]*BW for _ in range(BH)]
seen=set(); n=0; bad=0
START = int(sys.argv[2]) if len(sys.argv)>2 else 0
for s in S[START:]:
    try:
        x,y,dcs = strip_dc(s)
    except EOFError:
        bad+=1; continue
    if (x,y) in seen: break
    seen.add((x,y)); n+=1
    for k,(cr,cb,yv) in enumerate(dcs):
        bx = x//8 + (k % 8)
        by = y//8 + (k // 8)
        if 0 <= bx < BW and 0 <= by < BH:
            img[by][bx] = yv*16
            chroma[by][bx] = (cr*64, cb*64)
vals=[v for row in img for v in row]
lo,hi=min(vals),max(vals)
print(f"{n} strips, {bad} undecodable; luma DC range {lo}..{hi}")
span = max(hi-lo, 1)
out = Image.new("RGB",(BW,BH))
px = out.load()
for j in range(BH):
    for i in range(BW):
        v = int(255*(img[j][i]-lo)/span)
        cr,cb = chroma[j][i]
        r = max(0,min(255, v + (cr>>6)))
        b = max(0,min(255, v + (cb>>6)))
        px[i,j] = (r, v, b)
out = out.resize((BW*2, BH*2), Image.NEAREST)
out.save(sys.argv[1])
print("wrote", sys.argv[1])
