"""Draw original public handheld artwork; no development image is opened.
Requires Pillow only when regenerating the supplied PNG assets.
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
ROOT=Path(__file__).resolve().parent/'assets'
ROOT.mkdir(exist_ok=True)
S=2

def font(size):
    for name in ['segoeui.ttf','DejaVuSans.ttf']:
        try:return ImageFont.truetype(name,size*S)
        except OSError:pass
    return ImageFont.load_default(size=size*S)

def original():
    image=Image.new('RGBA',(1536*S,1024*S))
    draw=ImageDraw.Draw(image)
    def box(coords,fill,outline=None,width=1,radius=0):
        b=tuple(int(v*S) for v in coords)
        if radius:draw.rounded_rectangle(b,radius*S,fill=fill,outline=outline,width=width*S)
        else:draw.rectangle(b,fill=fill,outline=outline,width=width*S)
    def oval(coords,fill,outline=None,width=1):draw.ellipse(tuple(int(v*S) for v in coords),fill,outline,width*S)
    def line(points,fill,width=1):draw.line([(int(x*S),int(y*S)) for x,y in points],fill,width*S)
    def text(x,y,value,size=20,fill='#e4e5e6',anchor='mm'):draw.text((x*S,y*S),value,font=font(size),fill=fill,anchor=anchor)
    box((193,25,1343,1001),'#008fc8',radius=155)
    mask=Image.new('L',image.size);ImageDraw.Draw(mask).rounded_rectangle((202*S,34*S,1334*S,992*S),147*S,fill=255)
    gradient=Image.new('RGBA',image.size)
    gd=ImageDraw.Draw(gradient)
    for y in range(34*S,993*S):
        t=(y-34*S)/(959*S);c=tuple(int(a+(b-a)*t) for a,b in zip((76,78,82),(39,41,45)))+(255,)
        gd.line((202*S,y,1334*S,y),fill=c)
    image.alpha_composite(Image.composite(gradient,Image.new('RGBA',image.size),mask))
    draw=ImageDraw.Draw(image)
    box((208,40,1328,986),None,'#73767a',1,140)
    # Camera, speaker, and status light.
    oval((750,57,786,93),'#121418','#74777b',2)
    oval((760,67,777,84),'#07090d');oval((764,70,769,75),'#476071')
    for row in range(3):
        for col in range(3):oval((252+col*19,176+row*19,260+col*19,184+row*19),'#17191d')
    box((1209,154,1234,164),'#d1d7d8',radius=5)
    # Raised display surrounds. Screen rectangles preserve the existing layout.
    for x,y,w,h in [(463,145,610,348),(520,546,494,342)]:
        box((x-21,y-23,x+w+21,y+h+22),'#1a1b1e','#101114',4,13)
        box((x-9,y-10,x+w+9,y+h+10),'#111214','#777a7e',2,4)
        box((x,y,x+w,y+h),'#101010')
    # Circle pad and its recessed ring.
    oval((235,287,395,447),'#181b1f','#0d0f12',3)
    oval((244,296,386,438),'#7d8186','#9ea2a7',2)
    oval((257,309,373,425),'#b0b3b6','#34373c',4)
    oval((265,317,365,417),'#989da3','#d0d3d6',2)
    # D-pad with individually legible directional arms.
    cross=[(292,490),(340,490),(340,546),(380,546),(380,594),(340,594),(340,650),(292,650),(292,594),(250,594),(250,546),(292,546)]
    draw.polygon([(x*S,y*S) for x,y in cross],fill='#17191d',outline='#0c0e11',width=5*S)
    line([(299,640),(299,587),(260,587),(260,553),(299,553),(299,499),(333,499),(333,553),(371,553)],'#777c83',2)
    for points in [[(310,515),(321,515),(315,508)],[(310,625),(321,625),(315,633)],[(271,565),(271,577),(263,571)],[(358,565),(358,577),(367,571)]]:
        draw.polygon([(x*S,y*S) for x,y in points],fill='#b7bdc5')
    def button(x,y,r,label='',size=22):
        oval((x-r-3,y-r,x+r+3,y+r+6),'#111316')
        oval((x-r,y-r,x+r,y+r),'#33363b','#0d1014',3)
        oval((x-r+4,y-r+4,x+r-4,y+r-4),None,'#747980',1)
        if label:text(x,y,label,size)
    for x,y,label in [(1215,320,'X'),(1155,378,'Y'),(1277,378,'A'),(1215,440,'B')]:button(x,y,29,label,24)
    for y,label in [(595,'START'),(666,'SELECT')]:
        button(1165,y,19);text(1200,y,label,17,anchor='lm')
    button(768,950,37)
    draw.polygon([(746*S,949*S),(768*S,930*S),(790*S,949*S),(783*S,949*S),(783*S,969*S),(753*S,969*S),(753*S,949*S)],fill='#e5e8eb')
    box((764,951,773,970),'#555b62')
    button(1165,875,24)
    draw.arc((1152*S,862*S,1178*S,889*S),-65,245,fill='#e3e6e9',width=3*S)
    line([(1165,857),(1165,874)],'#e3e6e9',3)
    oval((312,861,320,869),'#07090c');text(316,893,'MIC',16)
    text(1209,942,'SLEEP  >',16)
    box((1136,963,1281,980),'#17191d','#797d82',1,2)
    for x in range(1142,1275,8):line([(x,967),(x,978)],'#6c7279',1)
    image.resize((1536,1024),Image.Resampling.LANCZOS).save(ROOT/'slate.png')
    return image

def fold():
    im=Image.new('RGBA',(2000,2000));d=ImageDraw.Draw(im)
    for bounds in [(190,70,1870,960),(190,1030,1870,1940)]:d.rounded_rectangle(bounds,110,fill='#45494e',outline='#009bd0',width=12)
    d.rounded_rectangle((260,965,1800,1030),25,fill='#25292e',outline='#7b8087',width=3)
    for x,y,w,h in [(450,225,1344,656),(558,1118,884,660)]:
        d.rounded_rectangle((x-20,y-20,x+w+20,y+h+20),12,fill='#17191d',outline='#959ba2',width=3);d.rectangle((x,y,x+w,y+h),fill='#101010')
    for x,y in [(1660,1350),(1580,1430),(1740,1430),(1660,1510)]:d.ellipse((x-35,y-35,x+35,y+35),fill='#25292e',outline='#a6adb5',width=3)
    d.polygon([(300,1400),(345,1400),(345,1450),(395,1450),(395,1495),(345,1495),(345,1545),(300,1545),(300,1495),(250,1495),(250,1450),(300,1450)],fill='#24282e',outline='#a6adb5',width=3)
    im.save(ROOT/'fold.png')

art=original();fold()
icon=Image.new('RGBA',(256,256));mini=art.resize((384,256),Image.Resampling.LANCZOS);icon.alpha_composite(mini,(-64,0));icon.save(ROOT/'icon.png')
