#define UNICODE
#define _UNICODE
#include <windows.h>
#include <commctrl.h>
#include <shellapi.h>
#include <shlobj.h>
#include <stdio.h>
#include <stdlib.h>
#include <wchar.h>

static void print_text(const wchar_t* text){int length=WideCharToMultiByte(CP_UTF8,0,text,-1,NULL,0,NULL,NULL);char* utf8=(char*)malloc(length);if(utf8){WideCharToMultiByte(CP_UTF8,0,text,-1,utf8,length,NULL,NULL);fputs(utf8,stdout);free(utf8);}}
static BOOL CALLBACK list_windows(HWND window, LPARAM data) {
 wchar_t name[512], title[1024];
 GetClassNameW(window,name,512); GetWindowTextW(window,title,1024);
 if(IsWindowVisible(window)){printf("window %p class=",window);print_text(name);fputs(" title=",stdout);print_text(title);puts("");}
 return TRUE;
}
static BOOL CALLBACK list_children(HWND window, LPARAM data) {
 wchar_t name[512], title[4096]; RECT rect;
 GetClassNameW(window,name,512); SendMessageW(window,WM_GETTEXT,4096,(LPARAM)title); GetWindowRect(window,&rect);
 printf("child %p id=%d class=",window,GetDlgCtrlID(window));print_text(name);printf(" visible=%d enabled=%d rect=%ld,%ld,%ld,%ld text=",IsWindowVisible(window),IsWindowEnabled(window),rect.left,rect.top,rect.right,rect.bottom);print_text(title);puts("");
 return TRUE;
}
int wmain(int argc,wchar_t **argv) {
 SetConsoleOutputCP(CP_UTF8);
 HWND window=FindWindowW(L"RoshanHeicOfflineWindow",NULL);
 if(argc<2)return 2;
 if(wcscmp(argv[1],L"windows")==0){EnumWindows(list_windows,0);return 0;}
 if(wcscmp(argv[1],L"measure")==0){
  HDC dc=CreateCompatibleDC(NULL);HFONT font=CreateFontW(-13,0,0,0,FW_NORMAL,0,0,0,DEFAULT_CHARSET,OUT_DEFAULT_PRECIS,CLIP_DEFAULT_PRECIS,CLEARTYPE_QUALITY,DEFAULT_PITCH,L"Segoe UI");HGDIOBJ old=SelectObject(dc,font);
  const wchar_t* names[]={L"\x202bپاک‌سازی\x202c",L"\x202bپوشه…\x202c",L"\x202bاولیه\x202c"};for(int i=0;i<3;i++){SIZE s;GetTextExtentPoint32W(dc,names[i],(int)wcslen(names[i]),&s);print_text(names[i]);printf(" width=%ld height=%ld\n",s.cx,s.cy);}SelectObject(dc,old);DeleteObject(font);DeleteDC(dc);return 0;
 }
 if(!window){fputs("main window not found\n",stderr);return 3;}
 if(wcscmp(argv[1],L"capture")==0 && argc>2){
  RECT r;POINT origin={0,0};GetClientRect(window,&r);ClientToScreen(window,&origin);int width=r.right-r.left,height=r.bottom-r.top;
  HDC screen=GetDC(NULL),memory=CreateCompatibleDC(screen);HBITMAP bitmap=CreateCompatibleBitmap(screen,width,height);HGDIOBJ old=SelectObject(memory,bitmap);
  BOOL copied=BitBlt(memory,0,0,width,height,screen,origin.x,origin.y,SRCCOPY|CAPTUREBLT);SelectObject(memory,old);
  BITMAPINFO info={0};info.bmiHeader.biSize=sizeof(BITMAPINFOHEADER);info.bmiHeader.biWidth=width;info.bmiHeader.biHeight=-height;info.bmiHeader.biPlanes=1;info.bmiHeader.biBitCount=32;info.bmiHeader.biCompression=BI_RGB;
  DWORD bytes=(DWORD)width*(DWORD)height*4;void* pixels=calloc(1,bytes);BOOL read=pixels&&GetDIBits(screen,bitmap,0,height,pixels,&info,DIB_RGB_COLORS);
  int result=13;if(copied&&read){FILE* file=_wfopen(argv[2],L"wb");if(file){BITMAPFILEHEADER header={0};header.bfType=0x4d42;header.bfOffBits=sizeof(header)+sizeof(BITMAPINFOHEADER);header.bfSize=header.bfOffBits+bytes;fwrite(&header,sizeof(header),1,file);fwrite(&info.bmiHeader,sizeof(BITMAPINFOHEADER),1,file);fwrite(pixels,bytes,1,file);fclose(file);printf("Captured native client region: %d x %d\n",width,height);result=0;}}
  free(pixels);DeleteObject(bitmap);DeleteDC(memory);ReleaseDC(NULL,screen);return result;
 }
 if(wcscmp(argv[1],L"cursor")==0){SetCursorPos(1180,880);return 0;}
 if(wcscmp(argv[1],L"rows")==0){
  DWORD pid=0;GetWindowThreadProcessId(window,&pid);HANDLE process=OpenProcess(PROCESS_VM_OPERATION|PROCESS_VM_WRITE|PROCESS_VM_READ,FALSE,pid);if(!process)return 10;
  SIZE_T bytes=sizeof(LVITEMW)+4096*sizeof(wchar_t);void* remote=VirtualAllocEx(process,NULL,bytes,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!remote){CloseHandle(process);return 11;}
  HWND list=GetDlgItem(window,110);int count=(int)SendMessageW(list,LVM_GETITEMCOUNT,0,0);
  for(int i=0;i<count;i++){printf("row=%d",i);for(int col=0;col<4;col++){LVITEMW item={0};item.iSubItem=col;item.cchTextMax=4096;item.pszText=(wchar_t*)((char*)remote+sizeof(LVITEMW));SIZE_T n=0;
   if(!WriteProcessMemory(process,remote,&item,sizeof(item),&n))continue;SendMessageW(list,LVM_GETITEMTEXTW,i,(LPARAM)remote);wchar_t text[4096]={0};ReadProcessMemory(process,item.pszText,text,sizeof(text)-sizeof(wchar_t),&n);printf("\t");print_text(text);}puts("");}
  VirtualFreeEx(process,remote,0,MEM_RELEASE);CloseHandle(process);return 0;
 }
 if(wcscmp(argv[1],L"pid")==0){DWORD pid=0;GetWindowThreadProcessId(window,&pid);printf("app_windows_pid=%lu\n",pid);return 0;}
 if(wcscmp(argv[1],L"delete")==0 && argc>2){SetLastError(0);BOOL ok=DeleteFileW(argv[2]);DWORD error=GetLastError();printf("DeleteFileW_success=%d GetLastError=%lu\n",ok,error);return 0;}
 if(wcscmp(argv[1],L"dialogbutton")==0 && argc>2){HWND dlg=FindWindowW(L"#32770",NULL);if(!dlg)return 5;HWND button=GetDlgItem(dlg,_wtoi(argv[2]));if(!button)return 7;PostMessageW(button,BM_CLICK,0,0);return 0;}
 if(wcscmp(argv[1],L"dialogok")==0){HWND dlg=FindWindowW(L"#32770",NULL);if(!dlg)return 5;PostMessageW(dlg,WM_COMMAND,IDOK,(LPARAM)GetDlgItem(dlg,IDOK));return 0;}
 if(wcscmp(argv[1],L"dialoginspect")==0){HWND dlg=FindWindowW(L"#32770",NULL);if(!dlg)return 5;EnumChildWindows(dlg,list_children,0);return 0;}
 if(wcscmp(argv[1],L"dialoginput")==0 && argc>2){
  HWND dlg=FindWindowW(L"#32770",NULL);if(!dlg)return 5;
  HWND combo=GetDlgItem(dlg,0x047c); HWND edit=combo?(HWND)SendMessageW(combo,CBEM_GETEDITCONTROL,0,0):NULL;
  if(!edit && combo)edit=FindWindowExW(combo,NULL,L"Edit",NULL);
  if(!edit)edit=GetDlgItem(dlg,0x0480); if(!edit)return 6;
  SetWindowTextW(edit,argv[2]); PostMessageW(dlg,WM_COMMAND,IDOK,(LPARAM)GetDlgItem(dlg,IDOK));return 0;
 }
 if(wcscmp(argv[1],L"last")==0 || wcscmp(argv[1],L"first")==0){HWND list=GetDlgItem(window,110);WPARAM key=wcscmp(argv[1],L"first")==0?VK_HOME:VK_END;SendMessageW(list,WM_KEYDOWN,key,0);SendMessageW(list,WM_KEYUP,key,0);return 0;}
 if(wcscmp(argv[1],L"inspect")==0){EnumChildWindows(window,list_children,0);return 0;}
 if(wcscmp(argv[1],L"command")==0 && argc>2){PostMessageW(window,WM_COMMAND,_wtoi(argv[2]),0);return 0;}
 if(wcscmp(argv[1],L"click")==0 && argc>2){PostMessageW(GetDlgItem(window,_wtoi(argv[2])),BM_CLICK,0,0);return 0;}
 if(wcscmp(argv[1],L"choose")==0 && argc>3){int id=_wtoi(argv[2]);HWND combo=GetDlgItem(window,id);SendMessageW(combo,CB_SETCURSEL,_wtoi(argv[3]),0);PostMessageW(window,WM_COMMAND,MAKEWPARAM(id,CBN_SELCHANGE),(LPARAM)combo);return 0;}
 if(wcscmp(argv[1],L"drop")==0 && argc>2){
  SIZE_T bytes=sizeof(DROPFILES)+sizeof(wchar_t);for(int i=2;i<argc;i++)bytes+=(wcslen(argv[i])+1)*sizeof(wchar_t);
  HGLOBAL memory=GlobalAlloc(GMEM_MOVEABLE|GMEM_ZEROINIT|GMEM_DDESHARE,bytes);if(!memory)return 8;
  DROPFILES* drop=(DROPFILES*)GlobalLock(memory);drop->pFiles=sizeof(DROPFILES);drop->fWide=TRUE;wchar_t* target=(wchar_t*)((char*)drop+sizeof(DROPFILES));
  for(int i=2;i<argc;i++){wcscpy(target,argv[i]);target+=wcslen(argv[i])+1;}*target=0;GlobalUnlock(memory);
  DWORD pid=0;GetWindowThreadProcessId(window,&pid);HANDLE process=OpenProcess(PROCESS_VM_OPERATION|PROCESS_VM_WRITE|PROCESS_VM_READ|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid);if(!process){GlobalFree(memory);return 10;}
  void* remote=VirtualAllocEx(process,NULL,bytes,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!remote){CloseHandle(process);GlobalFree(memory);return 11;}
  void* local=GlobalLock(memory);SIZE_T written=0;BOOL ok=WriteProcessMemory(process,remote,local,bytes,&written);GlobalUnlock(memory);GlobalFree(memory);
  if(!ok||written!=bytes){VirtualFreeEx(process,remote,0,MEM_RELEASE);CloseHandle(process);return 12;}
  SendMessageW(window,WM_DROPFILES,(WPARAM)remote,0);VirtualFreeEx(process,remote,0,MEM_RELEASE);CloseHandle(process);return 0;
 }
 if(wcscmp(argv[1],L"resize")==0 && argc>3){SetWindowPos(window,NULL,30,20,_wtoi(argv[2]),_wtoi(argv[3]),SWP_NOZORDER);return 0;}
 if(wcscmp(argv[1],L"close")==0){PostMessageW(window,WM_CLOSE,0,0);return 0;}
 if(wcscmp(argv[1],L"count")==0){printf("rows=%lld\n",(long long)SendMessageW(GetDlgItem(window,110),LVM_GETITEMCOUNT,0,0));return 0;}
 return 4;
}
