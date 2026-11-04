"""Original geometric CPF public artwork; no development artwork is read."""
from pathlib import Path
import struct, zlib
ROOT = Path(__file__).resolve().parent / 'assets'
def png(name, w, h, body, screens, buttons):
    pixels = bytearray(w*h*4)
    def rect(x,y,r,b,c):
        for row in range(max(y,0),min(b,h)):
            for col in range(max(x,0),min(r,w)):
                p=(row*w+col)*4; pixels[p:p+4]=bytes(c)
    rect(*body,(31,42,60,255))
    for x,y,sw,sh in screens:
        rect(x-12,y-12,x+sw+12,y+sh+12,(65,93,123,255))
        rect(x,y,x+sw,y+sh,(16,16,16,255))
    for x,y,r in buttons:
        for yy in range(y-r,y+r+1):
            for xx in range(x-r,x+r+1):
                if (xx-x)**2+(yy-y)**2<=r*r:
                    p=(yy*w+xx)*4; pixels[p:p+4]=bytes((82,142,177,255))
    if name == 'slate.png':
        rect(292,490,340,650,(82,142,177,255))
        rect(250,546,380,594,(82,142,177,255))
        glyphs={'A':['01110','10001','10001','11111','10001','10001','10001'],
                'B':['11110','10001','10001','11110','10001','10001','11110'],
                'X':['10001','10001','01010','00100','01010','10001','10001'],
                'Y':['10001','10001','01010','00100','00100','00100','00100'],
                'H':['10001','10001','10001','11111','10001','10001','10001'],
                'S':['01111','10000','10000','01110','00001','00001','11110'],
                'T':['11111','00100','00100','00100','00100','00100','00100'],
                'P':['11110','10001','10001','11110','10000','10000','10000']}
        for letter,x,y in [('X',1215,320),('Y',1155,378),('A',1277,378),('B',1215,440),('H',768,950),('S',1165,595),('T',1165,666),('P',1165,875)]:
            for gy,row in enumerate(glyphs[letter]):
                for gx,bit in enumerate(row):
                    if bit=='1': rect(x-10+gx*4,y-14+gy*4,x-6+gx*4,y-10+gy*4,(245,250,255,255))
    def chunk(t,d): return struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d)&0xffffffff)
    raw=b''.join(b'\0'+pixels[y*w*4:(y+1)*w*4] for y in range(h))
    data=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw,9))+chunk(b'IEND',b'')
    (ROOT/name).write_bytes(data)
ROOT.mkdir(exist_ok=True)
png('slate.png',1536,1024,(195,30,1340,995),[(463,145,610,348),(520,546,494,342)],[(1215,320,25),(1277,378,25),(1155,378,25),(1215,440,25),(315,570,38),(768,950,28),(1165,595,20),(1165,666,20),(1165,875,20)])
png('fold.png',2000,2000,(180,50,1850,1950),[(450,225,1344,656),(558,1118,884,660)],[(340,1350,50),(1660,1350,40),(1740,1430,40),(1580,1430,40),(1660,1510,40),(1000,1870,35)])
png('icon.png',64,64,(2,2,62,62),[(12,8,40,19),(17,34,30,19)],[])
