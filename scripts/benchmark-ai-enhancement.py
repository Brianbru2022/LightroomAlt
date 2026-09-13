"""Deterministic local enhancement timing/quality probe; no private images or network."""
from __future__ import annotations
import argparse,json,math,sys,time
from pathlib import Path
import numpy as np
from PIL import Image

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"ai-worker"))
from keepframe_worker import enhancement

def fixture(size:int)->Image.Image:
    y,x=np.mgrid[0:size,0:size];base=np.stack([(x*3+y)%256,(y*5+x//3)%256,((x//8+y//8)%2)*170+40],axis=2).astype(np.uint8)
    return Image.fromarray(base,"RGB")
def mse(a:Image.Image,b:Image.Image)->float:
    left=np.asarray(a,dtype=np.float32);right=np.asarray(b,dtype=np.float32);return float(np.mean((left-right)**2))
def psnr(a:Image.Image,b:Image.Image)->float:
    value=mse(a,b);return 99.0 if value==0 else 10*math.log10(255*255/value)
def run(image:Image.Image,operation:str,force_cpu:bool=False):
    started=time.perf_counter();result=enhancement.run_tile(image,operation,force_cpu=force_cpu);result["observedTotalMs"]=round((time.perf_counter()-started)*1000);result["pngBytes"]=len(result.pop("png"));return result
def main():
    parser=argparse.ArgumentParser();parser.add_argument("--denoise-size",type=int,default=256);parser.add_argument("--sr-size",type=int,default=128);parser.add_argument("--warm-runs",type=int,default=3);parser.add_argument("--cpu-smoke",action="store_true");args=parser.parse_args()
    clean=fixture(args.denoise_size);rng=np.random.default_rng(1601);noisy=Image.fromarray(np.clip(np.asarray(clean,dtype=np.int16)+rng.normal(0,12,np.asarray(clean).shape),0,255).astype(np.uint8),"RGB")
    enhancement.unload();cold_d=run(noisy,"denoise");denoised=Image.open(__import__("io").BytesIO(enhancement.run_tile(noisy,"denoise")["png"])).convert("RGB");warm_d=[run(noisy,"denoise") for _ in range(args.warm_runs)]
    low=fixture(args.sr_size);reference=low.resize((args.sr_size*4,args.sr_size*4),Image.Resampling.LANCZOS);enhancement.unload();cold_s=run(low,"super_resolution");sr=Image.open(__import__("io").BytesIO(enhancement.run_tile(low,"super_resolution")["png"])).convert("RGB");warm_s=[run(low,"super_resolution") for _ in range(args.warm_runs)]
    output={"hardware":enhancement.status(),"denoise":{"tile":[args.denoise_size,args.denoise_size],"cold":cold_d,"warm":warm_d,"quality":{"noisyMse":round(mse(clean,noisy),3),"outputMse":round(mse(clean,denoised),3)}},"superResolution":{"tile":[args.sr_size,args.sr_size],"output":[sr.width,sr.height],"cold":cold_s,"warm":warm_s,"quality":{"lanczosReferencePsnrDb":round(psnr(reference,sr),3)}}}
    if args.cpu_smoke:
        enhancement.unload();output["cpuDenoise32"]=run(fixture(32),"denoise",True)
    print(json.dumps(output,indent=2))
if __name__=="__main__":main()
