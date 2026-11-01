export interface Api {
  open(path: string): void;
}

export declare function connect(url: string): Api;
