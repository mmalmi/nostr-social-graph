export class NegentropyStorageVector {
 insert(timestamp:number,id:string):void;
 seal():void;
}
export class Negentropy {
 constructor(storage:NegentropyStorageVector,frameSizeLimit?:number);
 initiate():Promise<string>;
 reconcile(message:string):Promise<[string|null,string[],string[]]>;
}
