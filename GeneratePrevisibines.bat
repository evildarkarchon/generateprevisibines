@ECHO off
SETLOCAL
SETLOCAL ENABLEDELAYEDEXPANSION
:: ===============================================================================
:: Simple Batch procedure to build precombine/Previs using prompted Plugin "Seed"
::      (or xPrevisPatch.esp is used to creat33e that plugin if it doesn't exist)
:: Optional parameters [-clean/-filtered/-xbox] [-bsarch] [-FO4:directory] [modname.esp]
:: Parameter -clean, -filtered or -xbox change the build mode. Clean is default.
:: if -bsarch passed then BSarch.exe will be used instead of Archive2.exe.
:: if -FO4:directorypath passed then mods will be found in directorypath\data.
:: If passed modname.esp then that mod will get new Previsbines (doesn't prompt for mod or pause on completion).
::
:: If run from xEdit directory then that version of xEdit64.exe/FO4Edit64.exe/BSarch.exe will be used.
::
:: Author: PJM V2.93 Aug 2025 (Add -bsarch and -FO4:<dir> options, supports CKPE .toml, block unuseable modnames, fixes)

echo ============================================================================
echo Automatic Previsbine Builder V2.93
echo If you use MO2 then this must be run from within MO2
echo:
WHERE /Q reg.exe
IF %ERRORLEVEL% EQU 1 (echo ERROR - Windows Reg.exe cannot be found. You must Manually edit this script to set program Paths & goto PauseAndExit)

:: Find xEdit location. First try current directory
:xEditCheck
SET FO4Edit_=%~dp0FO4Edit64.exe
If Exist "%FO4Edit_%" goto FO4Check
SET FO4Edit_=%~dp0xEdit64.exe
If Exist "%FO4Edit_%" goto FO4Check
SET FO4Edit_=%~dp0FO4Edit.exe
If Exist "%FO4Edit_%" goto FO4Check
SET FO4Edit_=%~dp0xEdit.exe
If Exist "%FO4Edit_%" goto FO4Check

:: If that fails check the regsitry set by xEdit when first run.
FOR /F "tokens=2* skip=2" %%a in ('reg.exe query "HKCR\FO4Script\DefaultIcon" /v ""') do SET FO4Edit_=%%b

:FO4Check
:: Find Fallout4.exe/CK location (assumes FO4 has been started at least via standard launcher)
FOR /F "tokens=3* skip=2" %%a in ('reg.exe query "HKLM\SOFTWARE\Wow6432Node\Bethesda Softworks\Fallout4" /v "installed path"') do Set locCreationKit_=%%b

:: Test for Commandline Options
SET NoPrompt_=
Set ArchProg_=Archive2
SET BuildMode_=clean
If "%~1" NEQ "" Call :CheckParam "%~1"
If "%~2" NEQ "" Call :CheckParam "%~2"
If "%~3" NEQ "" Call :CheckParam "%~3"
If "%~4" NEQ "" Call :CheckParam "%~4"

:: Check we have everything we need to actually run this process properly...
SET CK_=CreationKit.exe
SET Archive_=%locCreationKit_%tools\archive2\archive2.exe
SET CKPEini_=CreationKitPlatformExtended.toml
SET CKPEHandleSetting_=bBSPointerHandleExtremly
SET CKPELogSetting_=sOutputFile
SET UnattenedLogfile_=%TEMP%\UnattendedScript.log

If Not Exist "%FO4Edit_%" (echo ERROR - FO4Edit/xEdit directory not found. Run this script from that directory & goto PauseAndExit)
If Not Exist "%locCreationKit_%Fallout4.exe" (echo ERROR - Fallout4.exe cannot be found. Run Fallout4Launcher.exe once to fix or use -FO4 parameter & goto PauseAndExit)

:: show the versions of everything
<nul set /p ="Using %FO4Edit_% V"
powershell "(Get-Item -path """%FO4Edit_%""").VersionInfo.ProductVersion"
<nul set /p ="Using %locCreationKit_%Fallout4.exe V"
powershell "(Get-Item -path """%locCreationKit_%Fallout4.exe""").VersionInfo.ProductVersion"

If Not Exist "%locCreationKit_%%CK_%" (echo ERROR - %CK_% cannot be found. Creation Kit must be installed & goto PauseAndExit)
If Not Exist "%locCreationKit_%winhttp.dll" (echo ERROR - CKPE not installed. You will not get a successful Patch without it & goto PauseAndExit)
If Not Exist "%Archive_%" (echo ERROR - Archive2.exe not found. Creation Kit not properly installed & goto PauseAndExit)

<nul set /p ="Using ... %CK_% V"
powershell "(Get-Item -path """%locCreationKit_%%CK_%""").VersionInfo.ProductVersion"
<nul set /p ="Using ... CKPE V"
powershell "(Get-Item -path """%locCreationKit_%winhttp.dll""").VersionInfo.ProductVersion"

:: Check if CK logging is redirected to a file. If no CKPE ini then try for old version
If Exist "%locCreationKit_%%CKPEini_%" goto TestCKPEConfig
SET CKPEini_=CreationKitPlatformExtended.ini
If Exist "%locCreationKit_%%CKPEini_%" goto TestCKPEConfig
SET CKPEini_=fallout4_test.ini
SET CKPEHandleSetting_=BSHandleRefObjectPatch
SET CKPELogSetting_=OutputFile
If NOT Exist "%locCreationKit_%%CKPEini_%" (echo ERROR - CKPE not installed properly. No settings file found & goto PauseAndExit)

:TestCKPEConfig
Echo Using ... %CKPEini_%
echo:
set CKlog_=
for /F "tokens=2 delims==#;" %%a in ('Findstr /I /C:"%CKPELogSetting_%=" "%locCreationKit_%%CKPEini_%"') do (
call :TrimLog %%a
)
IF "%CKlog_%" NEQ "" goto CheckCKPEConfig
echo ERROR - CK Logging not set in this ini. To fix, set %CKPELogSetting_%=CK.log in it.
goto PauseAndExit

:CheckCKPEConfig
Set CreationKitlog_=%locCreationKit_%%CKlog_%
If Exist "%locCreationKit_%steam_appid.txt" Set /p AppID_=< "%locCreationKit_%steam_appid.txt"
Echo Build Mode : %BuildMode_%
Echo Archiver   : %ArchProg_%.exe
Echo Steam AppID: %AppID_%
Echo CK Logfile : %CreationKitlog_%
echo:
Findstr /I /M /C:"%CKPEHandleSetting_%=true" "%locCreationKit_%%CKPEini_%" >nul
IF %ERRORLEVEL% EQU 0 goto CheckScriptVer
echo Increased Reference Limit not enabled, Precombine Step may fail.
echo To fix, set %CKPEHandleSetting_%=true in %CKPEini_%.
echo:

:CheckScriptVer
Call :CheckScripts "%FO4Edit_%" Batch_FO4MergePrevisandCleanRefr.pas V2.2
Call :CheckScripts "%FO4Edit_%" Batch_FO4MergeCombinedObjectsAndCheck.pas V1.5

If "%ArchProg_%" EQU "Archive2" goto ShowBanner
If Not Exist "%BSArchexe_%" (echo ERROR - BSArch.exe not found. xEdit not properly installed & goto PauseAndExit)
:ShowBanner
If "%NoPrompt_%" NEQ "" (
echo Building Previsbines for Patch %PluginName_%
echo =============================================================================
goto GotPlugin
)
echo Specify the name to call your Previs Patch (If no extension then assumes .esp)
echo If it does not exist then xPrevisPatch.esp will be renamed to it.
echo =============================================================================
echo:
:GetPlugin
If "%NoPrompt_%" NEQ "" goto PauseAndExit
SET PluginName_=__
SET /P PluginName_="Enter Patch Plugin name (return to exit): "
If "%PluginName_%" EQU "__" Exit
:GotPlugin
If /I "%BuildMode_%" NEQ "clean" Goto SkipSpace
SET PluginNoSpace_=%PluginName_: =%
If /I "%PluginNoSpace_%" NEQ "%PluginName_%" Goto SpaceInName
:SkipSpace
SET PluginExt_=%PluginName_:*.es=.es%
:: Assume no extension so use esp
SET PluginNameExt_=%PluginName_%.esp
If /I "%PluginExt_%" EQU "%PluginName_%" Goto CheckPluginName
:: Extension specified so remove it from patch name
SET PluginNameExt_=%PluginName_%
CALL SET "PluginName_=%%PluginNameExt_:%PluginExt_%=%%"

:: Dont allow reserved plugin names (used internally)
:CheckPluginName
If /I "%PluginName_%" EQU "previs" Goto BadPluginName
If /I "%PluginName_%" EQU "combinedobjects" Goto BadPluginName
If /I "%PluginName_%" NEQ "xprevispatch" Goto CheckPluginExists

:BadPluginName
echo ERROR - This plugin name is reserved, Please choose another.
echo:
Goto GetPlugin
:SpaceInName
echo ERROR - Plugin name cannot contain spaces, Please choose another.
echo:
Goto GetPlugin

:: No such Plugin so try and use Seed.
:TryCopySeed
If Exist "%locCreationKit_%Data\%PluginArchive_%" (echo ERROR - This Plugin already has an Archive & goto GetPlugin)
If "%NoPrompt_%" NEQ "" (echo ERROR - Plugin %PluginNameExt_% does not exist & goto GetPlugin)
If Not Exist "%locCreationKit_%Data\xPrevisPatch.esp" (echo ERROR - Specified Plugin and xPrevisPatch does not exist & goto GetPlugin)
CHOICE /C:YN /N /M "Plugin does not exist, Rename xPrevisPatch.esp to this? [Y/N]"
IF %ERRORLEVEL% NEQ 1 goto GetPlugin
Copy "%locCreationKit_%Data\xPrevisPatch.esp" "%PluginPath_%" > nul
:: Let MO2 do its thing before we check this worked...
If NOT EXIST "%PluginPath_%" timeout /t 5 >nul
If Exist "%PluginPath_%" goto Precomb
echo ERROR - Copy of xPrevisPatch to specified plugin failed - MO2 moved it?
goto PauseAndExit

:: If the specified mod already exists see what we want to do with it, otherwise use xPrevisPatch.esp as our seed.
:CheckPluginExists
echo:
SET PluginArchive_=%PluginName_% - Main.ba2
SET Logfile_=%temp%\%PluginName_%.log
SET PluginPath_=%locCreationKit_%Data\%PluginNameExt_%
If not Exist "%PluginPath_%" goto TryCopySeed
If "%NoPrompt_%" NEQ "" goto Precomb
CHOICE /C:YNC /N /M "Plugin already exists, Use It? [Y], Exit [N], Rerun from failed step [C]"
IF %ERRORLEVEL% EQU 1 goto Precomb
IF %ERRORLEVEL% NEQ 3 Exit

:GetStep
echo:
echo [1] Generate Precombines Via CK
echo [2] Merge PrecombineObjects.esp Via xEdit
echo [3] Create BA2 Archive from Precombines
If /I "%BuildMode_%" EQU "clean" (
echo [4] Compress PSG Via CK
echo [5] Build CDX Via CK
)
echo [6] Generate Previs Via CK
echo [7] Merge Previs.esp Via xEdit
echo [8] Add Previs files to BA2 Archive
CHOICE /C:123456780 /N /M "Restart at step (1 - 8 or 0 to exit): "
IF %ERRORLEVEL% EQU 1 goto RePrecomb
IF %ERRORLEVEL% EQU 2 goto PrecombMerge
IF %ERRORLEVEL% EQU 3 goto ArcPrecomb
IF %ERRORLEVEL% EQU 4 goto CompPSG
IF %ERRORLEVEL% EQU 5 goto BldCDX
IF %ERRORLEVEL% EQU 6 goto RePreVis
IF %ERRORLEVEL% EQU 7 goto PreVisMerge
IF %ERRORLEVEL% EQU 8 goto StartArchive
Goto CheckPluginExists

:RePrecomb
dir /a-d /s /b "%locCreationKit_%Data\meshes\precombined\*.nif" >nul 2>nul || goto Precomb1
CHOICE /C:YN /N /M "Precombine directory (Data\meshes\precombined) needs to be empty. Clean it? [Y/N]"
IF %ERRORLEVEL% NEQ 1 goto GetStep
RD /S /Q "%locCreationKit_%Data\meshes\Precombined"
goto Precomb1

:RePreVis
dir /a-d /s /b "%locCreationKit_%Data\vis\*.uvd" >nul 2>nul || goto PreVis1
CHOICE /C:YN /N /M "Previs directory (Data\vis) needs to be empty. Clean it? [Y/N]"
IF %ERRORLEVEL% NEQ 1 goto GetStep
RD /S /Q "%locCreationKit_%Data\vis"
goto PreVis1

:: Start precombine build process
:Precomb
dir /a-d /s /b "%locCreationKit_%Data\meshes\precombined\*.nif" >nul 2>nul || goto Precomb1
echo ERROR - Precombine directory (Data\meshes\precombined) not empty
Goto Done
:Precomb1
If Exist "%locCreationKit_%Data\%PluginArchive_%" (echo ERROR - This Plugin already has an Archive & goto GetPlugin)
dir /a-d /s /b "%locCreationKit_%Data\vis\*.uvd" >nul 2>nul || goto Precomb2
echo ERROR - Previs directory (Data\vis) not empty
Goto Done
:Precomb2
If Exist "%locCreationKit_%BSarchTemp" (RD /S /Q "%locCreationKit_%BSarchTemp")
If Exist "%locCreationKit_%Data\CombinedObjects.esp" (DEL /Q "%locCreationKit_%Data\CombinedObjects.esp")
If Exist "%locCreationKit_%Data\%PluginName_% - Geometry.psg" (DEL /Q "%locCreationKit_%Data\%PluginName_% - Geometry.psg")
If Exist "%Logfile_%" (DEL /Q "%Logfile_%")
ECHO 1 - Generating Precombines Via CK 
If /I "%BuildMode_%" EQU "clean" (
	Call :RunCK GeneratePrecombined CombinedObjects.esp "clean all"
	If not Exist "%locCreationKit_%Data\%PluginName_% - Geometry.psg" (echo ERROR - GeneratePrecombined failed to create psg file & goto failed)
) Else (
	Call :RunCK GeneratePrecombined CombinedObjects.esp "filtered all" )

dir /a-d /s /b "%locCreationKit_%Data\meshes\precombined\*.nif" >nul 2>nul || (echo ERROR - GeneratePrecombined failed to create any Precombines & goto failed)
If NOT Exist "%CreationKitlog_%" goto PrecombMerge
Findstr /I /M /C:"DEFAULT: OUT OF HANDLE ARRAY ENTRIES" "%CreationKitlog_%" >nul
IF %ERRORLEVEL% EQU 0 (echo ERROR - GeneratePrecombined ran out of Reference Handles & goto failed)

:: Merge CombinedObjects in Patch
:PrecombMerge
dir /a-d /s /b "%locCreationKit_%Data\meshes\precombined\*.nif" >nul 2>nul || (echo ERROR - No Precombined meshes found & goto PauseAndExit)
ECHO 2 - Merging PrecombineObjects.esp Via xEdit 
Call :RunScript "%FO4Edit_%" Batch_FO4MergeCombinedObjectsAndCheck.pas "%PluginNameExt_%" CombinedObjects.esp
Findstr /I /M /C:"Error: " "%UnattenedLogfile_%" >nul
IF %ERRORLEVEL% EQU 0 echo WARNING - Merge Precombines had errors

:: Archive precombines to get around file handle limit on CK
:ArcPrecomb
dir /a-d /s /b "%locCreationKit_%Data\meshes\precombined\*.nif" >nul 2>nul || Goto CompPSG
ECHO 3 - Creating BA2 Archive from Precombines
Call :Archive "meshes\precombined"
If "%ArchProg_%" EQU "Archive2" RD /S /Q "%locCreationKit_%Data\meshes\Precombined"

:: Compress Geometry (if Clean mode)
:CompPSG
If /I "%BuildMode_%" NEQ "clean" goto Previs
If not Exist "%locCreationKit_%Data\%PluginName_% - Geometry.psg" (echo ERROR - No Geometry file to Compress & goto failed)
ECHO 4 - Compressing PSG Via CK
Call :RunCK CompressPSG "%PluginName_% - Geometry.csg" ""
DEL /Q "%locCreationKit_%Data\%PluginName_% - Geometry.psg"

:: Build CDX (if Clean mode)
:BldCDX
If /I "%BuildMode_%" NEQ "clean" goto Previs
ECHO 5 - Building CDX Via CK
Call :RunCK BuildCDX "%PluginName_%.cdx" ""

:: Start the Previs build process
:PreVis
dir /a-d /s /b "%locCreationKit_%Data\vis\*.uvd" >nul 2>nul || goto PreVis1
echo ERROR - Previs directory (Data\vis) not empty
Goto Done
:PreVis1
If Exist "%locCreationKit_%Data\Previs.esp" (DEL /Q "%locCreationKit_%Data\Previs.esp")
ECHO 6 - Generating Previs Via CK
Call :RunCK GeneratePreVisData Previs.esp "clean all"
If NOT Exist "%CreationKitlog_%" goto PreVisMerge
Findstr /I /M /C:"ERROR: visibility task did not complete." "%CreationKitlog_%" >nul
IF %ERRORLEVEL% EQU 0 echo WARNING - GeneratePreVisData failed to build at least one Cluster uvd

:PreVisMerge
dir /a-d /s /b "%locCreationKit_%Data\vis\*.uvd" >nul 2>nul || (echo ERROR - No Visibility files Generated & goto failed)
If not Exist "%locCreationKit_%Data\Previs.esp" (echo ERROR - No Previs.esp Generated & goto failed)
ECHO 7 - Merging Previs.esp Via xEdit 
Call :RunScript "%FO4Edit_%" Batch_FO4MergePrevisandCleanRefr.pas "%PluginNameExt_%" Previs.esp
Findstr /I /M /C:"Completed: No Errors." "%UnattenedLogfile_%" >nul
IF %ERRORLEVEL% NEQ 0 echo WARNING - Merge Previs had errors

:: Do final Archive. No -add option so have to extract and re-archive

:StartArchive
dir /a-d /s /b "%locCreationKit_%Data\vis\*.uvd" >nul 2>nul || (echo WARNING - No Visibility files found to archive & goto Fin)
ECHO 8 - Adding Previs files to BA2 Archive
Call :AddToArchive vis
If "%ArchProg_%" EQU "Archive2" RD /S /Q "%locCreationKit_%Data\vis"

:Fin
echo Build of Patch %PluginName_% Complete. >> "%Logfile_%"
echo Build of Patch %PluginName_% Complete.
echo =====================================================
echo Patch Files created:
echo    %PluginNameExt_%
If /I "%BuildMode_%" EQU "clean" (
	echo    %PluginName_% - Geometry.csg
	echo    %PluginName_%.cdx
)
echo    %PluginArchive_%
echo:
echo Move ALL these files into a zip/7z archive and install it
echo =====================================================
If "%NoPrompt_%" NEQ "" goto Cleanup
CHOICE /C:YN /N /M "Remove working files [Y]? "
IF %ERRORLEVEL% NEQ 1 goto Done

:Cleanup
If Exist "%locCreationKit_%Data\CombinedObjects.esp" (DEL /Q "%locCreationKit_%Data\CombinedObjects.esp")
If Exist "%locCreationKit_%Data\Previs.esp" (DEL /Q "%locCreationKit_%Data\Previs.esp")
:Done
If Exist "%locCreationKit_%d3d11.dll-PJMdisabled" rename "%locCreationKit_%d3d11.dll-PJMdisabled" d3d11.dll
If Exist "%locCreationKit_%d3d10.dll-PJMdisabled" rename "%locCreationKit_%d3d10.dll-PJMdisabled" d3d10.dll
If Exist "%locCreationKit_%d3d9.dll-PJMdisabled" rename "%locCreationKit_%d3d9.dll-PJMdisabled" d3d9.dll
If Exist "%locCreationKit_%dxgi.dll-PJMdisabled" rename "%locCreationKit_%dxgi.dll-PJMdisabled" dxgi.dll
If Exist "%locCreationKit_%enbimgui.dll-PJMdisabled" rename "%locCreationKit_%enbimgui.dll-PJMdisabled" enbimgui.dll
If Exist "%locCreationKit_%d3dcompiler_46e.dll-PJMdisabled" rename "%locCreationKit_%d3dcompiler_46e.dll-PJMdisabled" d3dcompiler_46e.dll
echo See Log at %Logfile_%
If "%NoPrompt_%" NEQ "" goto :eof
:PauseAndExit
PAUSE
Exit
GOTO :eof

:Failed
echo Build of Patch %PluginName_% failed.
Goto Done

:: ========================= Functions ==================================
:: clean up the Logfile name %1
:TrimLog
set CKlog_=%~1
set CKlog_=!CKlog_:'=!
If /I "%CKlog_%" EQU "none" set CKlog_=
EXIT /B
GOTO :eof

:: Archive files %1
:Archive
SET Arch2Quals_=
If /I "%BuildMode_%" EQU "xbox" SET Arch2Quals_=-compression=XBox
echo Creating %Arch2Quals_% Archive %PluginArchive_% of %~1: >> "%Logfile_%"
echo ==================================== >> "%Logfile_%"
If "%ArchProg_%" EQU "Archive2" goto DoArchive2
IF Not Exist "%locCreationKit_%BSArchTemp\Meshes" MD "%locCreationKit_%BSArchTemp\Meshes" >nul
MOVE /Y "%locCreationKit_%Data\%~1" "%locCreationKit_%BSArchTemp\Meshes" >nul
START "BSArch" /D"%locCreationKit_%Data" /wait "%BSArchexe_%" Pack "%locCreationKit_%BSArchTemp" "%locCreationKit_%Data\%PluginArchive_%" -mt -fo4 -z >> "%Logfile_%"
IF %ERRORLEVEL% NEQ 0 (
echo ERROR - BSarch failed with error %ERRORLEVEL%
MOVE /Y "%locCreationKit_%BSArchTemp\%~1" "%locCreationKit_%Data\Meshes"  >nul
goto failed
)
Goto ArchiveDone
:DoArchive2
START "Archive" /D"%locCreationKit_%Data" /wait "%Archive_%" %~1 -c="%PluginArchive_%" %Arch2Quals_% -f=General -q >> "%Logfile_%"
IF %ERRORLEVEL% NEQ 0 (echo "ERROR - Archive2 failed with error %ERRORLEVEL%" & goto failed)
:ArchiveDone
If not Exist "%locCreationKit_%Data\%PluginArchive_%" (echo ERROR - No plugin archive Created & goto failed)
EXIT /B
GOTO :eof

:: Extract file from Archive
:Extract
echo Extracting Archive %PluginArchive_%: >> "%Logfile_%"
echo ==================================== >> "%Logfile_%"
START "Extract" /D"%locCreationKit_%Data" /wait "%Archive_%" "%PluginArchive_%" -e=. -q >> "%Logfile_%"
IF %ERRORLEVEL% NEQ 0 (echo "ERROR - Archive2 Extract failed with error %ERRORLEVEL%" & goto failed)
EXIT /B
GOTO :eof

:: Add files %1 into existing Archive of Precombines
:: Unfortunately Archive2 does not support this so must extract and re-archive.
:AddToArchive
If not Exist "%locCreationKit_%Data\%PluginArchive_%" GOTO ArchiveOnly
If "%ArchProg_%" EQU "Archive2" goto AddToArchive2
echo Creating %Arch2Quals_% Archive %PluginArchive_% of meshes\precombined,%~1: >> "%Logfile_%"
echo ==================================== >> "%Logfile_%"
IF Not Exist "%locCreationKit_%BSArchTemp" MD "%locCreationKit_%BSArchTemp" >nul
MOVE /Y "%locCreationKit_%Data\%~1" "%locCreationKit_%BSArchTemp" >nul
START "BSArch" /D"%locCreationKit_%Data" /wait "%BSArchexe_%" Pack "%locCreationKit_%BSArchTemp" "%locCreationKit_%Data\%PluginArchive_%" -mt -fo4 -z >> "%Logfile_%"
IF %ERRORLEVEL% NEQ 0 (
echo ERROR - BSArch failed with error %ERRORLEVEL%
MOVE /Y "%locCreationKit_%BSArchTemp\%~1" "%locCreationKit_%Data" >nul
goto failed)
RD /S /Q "%locCreationKit_%BSArchTemp"
Goto AddToArchiveDone
:AddToArchive2
Call :Extract
timeout /t 5 >nul
DEL /Q "%locCreationKit_%Data\%PluginArchive_%"
dir /a-d /s /b "%locCreationKit_%Data\meshes\precombined\*.nif" >nul 2>nul || GOTO ArchiveOnly
Call :Archive "meshes\precombined,%1"
RD /S /Q "%locCreationKit_%Data\meshes\Precombined"
GOTO AddToArchiveDone
:ArchiveOnly
Call :Archive %1
:AddToArchiveDone
EXIT /B
GOTO :eof

:: ===========================================================
:: Run CK option %1 to generate %2 with qualifiers %3
:RunCK
If Exist "%locCreationKit_%d3d11.dll" rename "%locCreationKit_%d3d11.dll" d3d11.dll-PJMdisabled
If Exist "%locCreationKit_%d3d10.dll" rename "%locCreationKit_%d3d10.dll" d3d10.dll-PJMdisabled
If Exist "%locCreationKit_%d3d9.dll" rename "%locCreationKit_%d3d9.dll" d3d9.dll-PJMdisabled
If Exist "%locCreationKit_%dxgi.dll" rename "%locCreationKit_%dxgi.dll" dxgi.dll-PJMdisabled
If Exist "%locCreationKit_%enbimgui.dll" rename "%locCreationKit_%enbimgui.dll" enbimgui.dll-PJMdisabled
If Exist "%locCreationKit_%d3dcompiler_46e.dll" rename "%locCreationKit_%d3dcompiler_46e.dll" d3dcompiler_46e.dll-PJMdisabled
If Exist "%CreationKitlog_%" DEL /Q "%CreationKitlog_%"
echo Running CK option %1: >> "%Logfile_%"
echo ==================================== >> "%Logfile_%"
ECHO Start %time% >> "%Logfile_%"
START "CK" /D"%locCreationKit_%" /wait "%CK_%" -%1:"%PluginNameExt_%" %~3
Set Err_=%ERRORLEVEL%
ECHO Ended %time% >> "%Logfile_%"
:: Give MO2 time to move files around.
timeout /t 10 >nul
If Not Exist "%CreationKitlog_%" ECHO Unable to find log  %CreationKitlog_% >> "%Logfile_%"
If Exist "%CreationKitlog_%" type "%CreationKitlog_%" >> "%Logfile_%"
If not Exist "%locCreationKit_%Data\%~2" (echo "ERROR - %1 failed to create file %~2 with exit status %Err_%" & goto failed)
IF %Err_% NEQ 0 echo WARNING - %1 ended with error %Err_% but seemed to finish so error ignored.
EXIT /B
GOTO :eof

:: Process Command Line Parameter
:CheckParam
If /I "%~1" EQU "-bsarch" goto SetArchMode
If /I "%~1" EQU "filtered" goto SetBuildMode
If /I "%~1" EQU "-filtered" goto SetBuildMode
If /I "%~1" EQU "xbox" goto SetBuildMode
If /I "%~1" EQU "-xbox" goto SetBuildMode
If /I "%~1" EQU "clean" goto SetBuildMode
If /I "%~1" EQU "-clean" goto SetBuildMode
SET dir_=%~1
SET dir_=%dir_:-fo4:=%
If "%dir_%" EQU "%~1" goto SetPlugin
SET locCreationKit_=%dir_%\
Goto CheckParamDone

:SetArchMode
Set ArchProg_=BSArch
goto CheckParamDone

:SetPlugin
SET PluginName_=%~1
SET NoPrompt_=true
Goto CheckParamDone

:SetBuildMode
SET BuildMode_=%~1
SET BuildMode_=%BuildMode_:-=%
:CheckParamDone
EXIT /B
GOTO :eof

:: Check required scripts exists
:CheckScripts
If Not EXIST "%~dp1Edit Scripts\%2" (echo ERROR - Required xEdit Script %2 missing & goto PauseAndExit)
SET BSArchexe_=%~dp1BSArch.exe
Findstr /I /M /C:"%3" "%~dp1Edit Scripts\%2" >nul
If %ERRORLEVEL% NEQ 0 (echo ERROR - Old Script %2 found, %3 required & goto PauseAndExit)
EXIT /B
GOTO :eof

:: ===========================================================
:: Run xEdit %1 with script %2 against plugins %3 %4
:RunScript
SET LocPlugins_=%TEMP%\Plugins.txt
SET xEditProc_=%~n1

Echo *%~3 > "%LocPlugins_%"
Echo *%~4 >> "%LocPlugins_%"

If EXIST "%UnattenedLogfile_%" DEL /Q "%UnattenedLogfile_%"

echo Running xEdit script %2 against %~3 >> "%Logfile_%"
echo ==================================== >> "%Logfile_%"
timeout /t 10 >nul
START "xEdit" /B %1 -fo4 -autoexit -P:"%LocPlugins_%" -Script:%2 -Mod:%3 -log:"%UnattenedLogfile_%"
:: Send keypresses to xtart xEdit processing
timeout /t 5 >nul
Powershell -command "& {$wshell = New-Object -ComObject wscript.shell;$wshell.AppActivate('%xEditProc_%');Start-Sleep -s 1;$wshell.AppActivate('Module Selection');$wshell.SendKeys('{ENTER}')}"  >nul
:: wait for script to finish processing and create Log
:loop
timeout /t 5 >nul
If NOT EXIST "%UnattenedLogfile_%" GOTO loop
:: Close the xEdit window as it does not Autoclose
timeout /t 10 >nul
powershell (ps %xEditProc_%).CloseMainWindow() >nul
:: That sometimes fails so do a last ditch kill.
timeout /t 15 >nul
TaskKill /IM %xEditProc_%.exe 2>nul
:: Give MO2 time to move files around.
timeout /t 10 >nul
type "%UnattenedLogfile_%" >> "%Logfile_%"
Findstr /I /M /C:"Completed: " "%UnattenedLogfile_%" >nul
If %ERRORLEVEL% NEQ 0 (echo ERROR - FO4Edit script %2 failed & goto failed)
EXIT /B
GOTO :eof