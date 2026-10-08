mkdir -p fixtures/rive/ref && cd fixtures/rive/ref
rive --version > version.txt
rive docs format > docs-format.txt
rive docs gotchas > docs-gotchas.txt
rive docs --list > docs-list.txt
for t in Artboard Shape Rectangle Ellipse PointsPath StraightVertex CubicMirroredVertex \
         Fill Stroke SolidColor LinearGradient RadialGradient GradientStop TrimPath ClippingShape \
         Node LinearAnimation KeyedObject KeyedProperty KeyFrameDouble KeyFrameColor \
         CubicEaseInterpolator NestedArtboard; do
  rive schema $t --json > schema-$t.json 2>&1
done
cp -R "$(rive samples --path)/rml_triangle" sample_triangle
