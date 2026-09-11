import {StringDecoder} from "node:string_decoder";
import {redact} from "./events.ts";
// Redact complete lines so credentials split across output chunks cannot leak.
// Oversized lines are discarded rather than streaming an unvalidated prefix.
export class LogRedactor {
  private decoder=new StringDecoder("utf8");
  private pending="";
  private dropping=false;
  constructor(privateSecrets:string[],write:(text:string)=>void){this.secrets=privateSecrets;this.write=write;}
  private secrets:string[];
  private write:(text:string)=>void;
  push(chunk:Buffer){this.consume(this.decoder.write(chunk));}
  private consume(text:string){
    const lines=text.split("\n");
    for(let i=0;i<lines.length;i++){
      if(!this.dropping)this.pending+=lines[i];
      if(this.pending.length>65536){this.pending="";this.dropping=true;}
      if(i<lines.length-1){this.write(this.dropping?"[oversized log line omitted]\n":redact(this.pending,this.secrets)+"\n");this.pending="";this.dropping=false;}
    }
  }
  end(){this.consume(this.decoder.end());if(this.pending||this.dropping)this.write(this.dropping?"[oversized log line omitted]\n":redact(this.pending,this.secrets));this.pending="";}
}
