import test from "node:test";
import assert from "node:assert/strict";
import {LogRedactor} from "../lib/log-redactor.ts";
test("split credentials and oversized lines never reach persisted output",()=>{
  let output="";const redactor=new LogRedactor(["hidden-credential"],text=>{output+=text;});
  redactor.push(Buffer.from("normal hidden-"));redactor.push(Buffer.from("credential\npassword=also-"));
  redactor.push(Buffer.from("private\n"+"x".repeat(65537)));redactor.push(Buffer.from("\nfinish"));redactor.end();
  assert.equal(output,"normal [REDACTED]\npassword=[REDACTED]\n[oversized log line omitted]\nfinish");
});
