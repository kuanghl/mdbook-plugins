# pdf-viewer

## pdf as a component

[线性代数应该这样学-第四版](./线性代数应该这样学-第四版.pdf "web-preview")

## draw support

- all

```sh
# mermaid
mmdr input.mmd -o output.svg

# latex only pdf
tectonic -X build

# typst
typst compile --format pdf input.typ

# plantuml svg
plantuml-little diagram.puml
```

- plantuml code

```yaml
@startyaml
doe: "a deer, a female deer"
ray: "a drop of golden sun"
pi: 3.14159
xmas: true
french-hens: 3
calling-birds: 
	- huey
	- dewey
	- louie
	- fred
xmas-fifth-day: 
	calling-birds: four
	french-hens: 3
	golden-rings: 5
	partridges: 
		count: 1
		location: "a pear tree"
	turtle-doves: two
@endyaml
```