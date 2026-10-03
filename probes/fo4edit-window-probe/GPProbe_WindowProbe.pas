{
  Throwaway xEdit script for the FO4Edit window probe (map ticket #40).

  Mirrors the structure of PJM's Batch_FO4MergeCombinedObjectsandCheck.pas so the probe sees the
  same timing as a real Step 2/7 run: it parses -mod: and -log: from ParamStr with the same
  60-character copy, changes the named (already existing) plugin in memory, and writes the
  whole log in one SaveToFile at the end of Initialize. Saving the plugin is left to xEdit's
  close path, exactly as in the PJM scripts.
}
unit GPProbe_WindowProbe;

var
	Modname, Logname: string;
	Logfile: TStringList;
	Logging: Boolean;

function Initialize: Integer;
var
	i, j: integer;
	str: string;
	modfile, grp, rec: IInterface;
begin
	Result := 1;
	Modname := '';
	Logging := false;
	for i := 0 to paramcount do begin
		str := lowercase(paramstr(i));
		j := pos('-mod:', str);
		if j > 0 then
			Modname := trim(StringReplace(copy(str, j + 5, 60), #34, '', [rfReplaceAll]));
		j := pos('-log:', str);
		if j > 0 then begin
			Logname := trim(StringReplace(copy(str, j + 5, 60), #34, '', [rfReplaceAll]));
			Logging := true;
		end;
	end;
	if Logging then Logfile := TStringList.Create;
	LogMsg('GPProbe window probe script, mod=[' + Modname + '] log=[' + Logname + ']');
	for i := 0 to FileCount - 1 do
		if LowerCase(GetFileName(FileByIndex(i))) = Modname then
			modfile := FileByIndex(i);
	if not Assigned(modfile) then
		LogMsg('Error: Missing [' + Modname + '] module')
	else begin
		// A new GLOB record marks the plugin modified, so xEdit's close path must save it.
		grp := Add(modfile, 'GLOB', True);
		rec := Add(grp, 'GLOB', True);
		SetElementEditValues(rec, 'EDID', 'GPProbeMarker');
		LogMsg('Completed: GPProbeMarker added to ' + Modname);
	end;
	if Logging then begin
		Logfile.SaveToFile(Logname);
		Logfile.Free;
	end;
end;

procedure LogMsg(str: string);
begin
	if Logging then Logfile.Add(str);
	AddMessage(str);
end;

function Finalize: Integer;
begin
end;

end.
