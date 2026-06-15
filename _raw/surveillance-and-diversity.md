Surveillance—the collection, monitoring and analysis of data on people—is a ubiquitous part of modern life1–3. Technologies have transformed the scale and magnitude of surveillance, transforming and exacerbating inequalities in many domains of social life4,5, including citizenship6–9, the workplace10, the criminal legal system11,12 and family life13,14. Cameras overseeing public spaces are an increasingly prevalent form of surveillance15; indeed, cameras are now so commonplace as to become a ‘banal good’16. With the ostensible purpose of providing information on when and by whom neighborhood norms, rules or laws are violated, surveillance cameras are used by private businesses and individuals as well as by police and other officials to monitor public space. Thus, surveillance cameras become an apparatus of social control, or the means by which individuals and institutions regulate behavior to maintain social order17.

Surveillance camera allocation is determined by a confluence of stakeholders, including police, legislators and the public18, and cameras are intended to be allocated to maximize effectiveness in preventing and deterring crime and disorder19,20. While surveillance cameras may provide benefits by deterring crime and enhancing residents’ perceived safety21, they also come with costs. If cameras are concentrated in certain neighborhoods, even after adjusting for crime rates, this may contribute to unequal exposure to surveillance, reinforcing disparities in privacy and policing. Such disparities can shape neighborhood experiences and deepen racialized inequalities in urban neighborhoods.

Existing theory provides competing expectations of the relationship between surveillance cameras and the racial composition of a neighborhood. Theories of the carceral state highlight that Black residents are enmeshed in a ‘carceral continuum’ wherein the neighborhoods in which they live are extensions of the carceral system, thus becoming increasingly penalized and surveilled through various means22,23. Surveillance, namely by the police, is but one tool to regulate and control Black neighborhoods and their residents22–24. In support of this, existing research finds that areas with more Black residents experience greater social control through various state apparatuses, such as increased arrests, police presence and police violence25–29. Therefore, according to the carceral state hypothesis, cameras would be most prevalent in neighborhoods with more Black residents30–32.

Alternatively, surveillance cameras may be fewer in already racially homogeneous neighborhoods. Racial threat theory suggests that dominant groups use various methods to control groups threatening...
their interests. However, in racially segregated neighborhoods, where minority groups are already isolated from the dominant group, additional social control measures may be deemed unnecessary. Rather than policing Black neighborhoods, surveillance cameras may reflect a dominant group’s boundary-making within a racially heterogeneous neighborhood. In racially diverse gentrifying neighborhoods, white householders may place particular emphasis on enforcing social order as defined by their norms and values, often at the cost of the other residents in a neighborhood. While some residents of diverse neighborhoods may perceive their communities as harmonious, for white householders, racial diversity may lead to an erosion of trust. This dynamic is particularly evident in the context of gentrification, where white gentrifiers are drawn to diverse neighborhoods but nonetheless institute social control mechanisms in their new neighborhoods. This emphasis on social order and erosion of trust in diverse areas may lead white householders to advocate for or install surveillance cameras to monitor those they perceive as outsiders. Thus, camera variation may represent a desire to make more racially diverse areas feel safer, more ‘habitable’ and adequately ‘defended’ for white residents.

We note that, while our analysis focuses on surveillance cameras in the context of increasing racial diversity in gentrifying neighborhoods, we recognize that surveillance can take different forms in other neighborhood change contexts, such as Black gentrification of Black neighborhoods. We engage with these distinctions further in the Discussion.

To examine these expectations, we need to know where surveillance cameras are, but data were previously limited. In this Article, we test the relationship between neighborhood racial composition and change and surveillance camera presence by extending data and methods developed by Sheng et al. for surveillance camera identification. Specifically, our analysis focuses on traditionally mounted surveillance cameras, such as those affixed to buildings, poles and streetlights, that are visible in Google Street View (GSV) imagery. Residential doorbell cameras (for example, Ring and Nest), which are not often visible in street-level imagery, are not included in our analysis.

Whereas Sheng et al. identified camera prevalence and observed, but did not explore, a U-shaped relationship between the presence of minority groups and camera prevalence, our study investigates the sociological and demographic patterns behind these patterns by testing competing theoretical expectations about racialized surveillance. Using computer vision and human verification with a large corpus of GSV images, crime data and Census and American Community Survey data, we estimate how the camera prevalence and increase relates to the racial composition and diversity of neighborhoods in the ten most densely population US cities with at least 500,000 residents.

Using these data, we answer two main research questions. First, cross-sectionally, how does the presence and number of surveillance cameras relate to neighborhood racial demographics? In particular, we advance existing empirical and theoretical literature by considering how racial composition and diversity matter in a neighborhood. Second, how are changes in neighborhoods’ racial composition and diversity associated with increases in camera density?

We observe a negative relationship between the share of Black residents in a neighborhood and camera prevalence. However, rather than simply being concentrated in all non-Black or predominantly white neighborhoods, we find that cameras are most prevalent where racial diversity is high, and diversity relates to camera prevalence even when conditioning on crime. Furthermore, as neighborhoods become more diverse, there is an increase in average camera density, associated with the entrance of white residents into non-white neighborhoods.

Our findings highlight how surveillance cameras are most present in racially heterogeneous neighborhoods, challenging notions of harmonious, diverse neighborhoods. Furthermore, our results underscore the value of computational methods and visual data in understanding spatial inequality.

Results

Bivariate relationships

We begin by presenting the bivariate relationship between camera prevalence, the presence of white and Black residents and neighborhood diversity (Fig. 1). In the figure, we present natural spline fits with two degrees of freedom, which allows us to better capture nonlinearities in the data. We also present the linear fit in Supplementary Fig. 4.

In Fig. 1 (left), the spline regression fit reveals a quadratic relationship between the share of a neighborhood that is Black and the camera identification rate. Cameras are most frequently detected in neighborhoods where approximately 25% of residents are Black, with identification rates (the density of cameras detected per block group, as further detailed in the Methods) tapering off in areas with both lower and higher percentages of Black residents.

In Fig. 1 (middle), the spline regression fit shows a similar quadratic relationship between the percentage of white residents in a census block group (our operationalization of a neighborhood) and the camera identification rate. The highest camera identification rates are observed in neighborhoods where about 50% of residents are white, confirming earlier findings by Sheng et al. Next, Fig. 1 (right) illustrates the relationship between camera prevalence and block-group diversity. The spline fit demonstrates a clear positive, monotonic relationship: more diverse neighborhoods tend to have higher rates of camera detection.

Cross-sectional regression results

We now further explore these relationships using multivariate regression models to account for additional neighborhood characteristics and potential confounders. Figure 2 illustrates the estimated camera identification rate as a function of the racial composition of neighborhoods, specifically focusing on the percentage of non-Hispanic Black and non-Hispanic white residents and crime rates (see the corresponding table, Supplementary Table 2). Given the parabolic relationships observed in the bivariate models, this model includes quadratic terms for both Black and white residents. The predictions are derived from a zero-inflated Poisson regression model, and the shaded areas represent 95% confidence intervals. Continuous independent variables are standardized to a zero mean and unit standard deviation within each city.

Figure 2a (left) shows the estimated relationship between the percentage of non-Hispanic Black residents in a neighborhood and the camera identification rate conditional on the crime rate. Here, we observe a negative association: neighborhoods with a higher percentage of non-Hispanic Black residents have lower camera identification rates after accounting for crime rates.

Conversely, Fig. 2a (middle) reveals a nonlinear relationship between the percentage of non-Hispanic white residents and the estimated camera identification rate: the camera identification rate is constant with the percentage of non-Hispanic white residents up to a certain point (around the city mean) after which it starts to decline, although estimates are noisy. Thus, neighborhoods with low-to-moderate proportions of white residents appear to have higher camera detection rates than those with very high proportions of white residents. Finally, Fig. 2a (right) shows the relationship between crime and camera identification rates. As expected, areas with higher total crime rates exhibit higher camera identification rates, even conditional on the racial composition.

Next, in a separate model in Fig. 2b, we test how the diversity of the larger spatial context relates to camera prevalence conditional on crime. As shown, camera detection is monotonically positively related to neighborhood diversity, indicating that more diverse neighborhoods tend to have higher rates of camera detection. In fact, as shown in Fig. 2b (right), the relationship is stronger between diversity and cameras than between crime and cameras, the ostensible purpose of cameras. The positive relationship between diversity and cameras holds even when controlling for reported crime rates, so we suspect...
that cameras may be used as a means of protection or threat in racially heterogeneous areas.

Because of the observed relationship between diversity and camera placement, we next investigate how the relationship between racial composition and camera prevalence varies across different levels of diversity. We examine whether unique racial dynamics emerge in diverse settings that could influence where cameras are placed. For instance, in more diverse neighborhoods, the presence of white or Black residents might lead to distinct social interactions, perceptions of safety or collective actions that affect camera prevalence differently than in more homogeneous areas. Thus, next, we test if neighborhood diversity moderates the relationship between the (residualized) shares of white and Black residents and camera identification rates in Fig. 2c.

In Fig. 2c (left), the results indicate that areas with lower diversity exhibit a negative relationship between the percentage of Black residents and the camera identification rate. By contrast, where diversity is high, there is a positive relationship between camera identification rates and the percentage of Black residents. Figure 2c (right) shows that the relationship between the percentage of white residents and the camera identification rate is more pronounced in areas with higher diversity. Higher diversity strengthens the positive relationship, while lower diversity weakens it.

Note that, given that we do not control for the share of Hispanic and Asian residents in a neighborhood in our models, it may be that our entropy measure is capturing merely the presence of these groups. Thus, in Supplementary Table 1, we present the correlations between entropy and the percentages of Black, white, Hispanic and Asian residents within each city and in the overall sample, showing that overall correlations are lower than 0.500 for all groups. However, because of the slightly high correlation between the percentage of Asian residents and entropy (0.414), we also include a robustness check (Supplementary Table 3) that controls for the Asian share in a neighborhood when considering entropy. Results are robust to the inclusion of this control.

Altogether, these results undermine the carceral state hypothesis as it relates to surveillance cameras. Diversity, more so than the share of Black residents, is related to camera placement and modifies how racial composition matters. In the most diverse areas in a city, the shares of Black and white residents relate positively to camera presence. These findings may indicate two alternative stories: surveillance cameras may be used for social control in ethnoracially diverse areas by white householders as suggested by previous research, or diversity fosters coalition-building for safety, where residents in more diverse neighborhoods collectively advocate for cameras. While we cannot directly determine who advocates for or installs cameras, we next consider the dynamic process of increasing cameras within a neighborhood to adjudicate between these explanations.

**Neighborhood change results**

To further explore the relationship between diversity and cameras, we present the expected change in camera identification rate as a function of changes in neighborhood diversity (Fig. 3 and Supplementary Table 5). Overall, change in diversity, as measured by entropy, is positively related to the probability of gaining cameras within a block group. This finding provides insight into why cameras are most prevalent in more diverse areas; as neighborhoods become more diverse, more cameras are situated and, thus, surveillance is entrenched in the most diverse neighborhoods.

While we do not directly observe who advocates for the installation of cameras, to understand which racial changes in the neighborhood might explain this relationship, we analyze whether an influx of white or Black residents is associated with increased camera installation in Fig. 3. We present the expected change in camera identification rates as a function of changes in the share of white (Fig. 3a) and the share of Black (Fig. 3b) residents in the neighborhood.

For changes in the white population, we examine the interaction with the baseline presence of non-white residents. As shown in Fig. 3a, in neighborhoods with large preexisting non-white shares, greater increases in white residents relate to increases in camera identification rates. However, in areas with lower baseline non-white residence, this relationship is weaker. In Fig. 3b, we focus on the interaction between the changes in the Black population and the baseline presence of white residents. This focus highlights the potential role of white householders in camera allocation, the presumed dominant group in the neighborhood (we also explore the interaction between non-white and Black population changes, detailed in Supplementary Table 6). We observe an overall negative relationship between the increase in Black residents and camera prevalence, which does not vary by the baseline presence of white residents.

Therefore, in combination with the earlier results, this suggests that the increases in diversity that lead to more camera prevalence are driven by white residents moving into non-white neighborhoods. This suggests that new white residents are using surveillance cameras as a means of social control in the neighborhoods that they move into, undermining a coalition-building argument.
To investigate the role of nearby neighborhood changes in surveillance camera prevalence, we also estimated models incorporating spatial lags of the change in diversity and the shares of white and Black residents. As detailed in Supplementary Table 10, these results show that increases in racial diversity in nearby neighborhoods are associated with increases in cameras and increases in nearby Black residents are related to decreases in cameras. However, increases in the share of white residents in nearby neighborhoods are unrelated to a neighborhood’s increase in cameras.

**Discussion**

For decades, scholars of surveillance have discussed the disproportionate, racialized use of surveillance technologies. However, the number, placement and growth of surveillance cameras, key parts of the surveillance apparatus in the USA, have been hitherto unknown. By utilizing advances in computer vision with longitudinal street-level imagery, we estimate where surveillance cameras are and how their placement relates to neighborhood racial composition. Our results indicate that neighborhood diversity is an important determinant of where cameras are, beyond the share of Black residents in a neighborhood. Notably, surveillance cameras are most common in racially diverse neighborhoods experiencing an influx of white residents, suggesting that white householders are instituting this means of social control as they move into or gentrify non-white neighborhoods.

Contrary to expectations drawn from prior research on the carceral state, we do not find evidence of disproportionate camera prevalence in Black neighborhoods. In fact, we find that Black neighborhoods have fewer surveillance cameras than comparable neighborhoods. This suggests that surveillance cameras differ from other forms of surveillance, such as increased police presence, which have historically targeted Black communities. Surveillance cameras may, instead, be a neighborhood amenity that is either refused by or denied to Black residents.

---

**Fig. 2** Estimated camera identification rate for various models. **a**, The relationships for a model including racial composition and crime. **b**, The relationships for a model including diversity and crime. **c**, The relationships for a model testing how diversity moderates the relationship between camera identification for shares of Black and white. The shaded areas represent 95% confidence intervals. Throughout, the estimated values were obtained from zero-inflated Poisson regression models adjusted for modal zone, population, household income, housing vacancy rate, median home value, city fixed effects and road length, with the log of image count used as weights.
neighborhoods, while being embraced or advocated for in more diverse settings with more Black householders. We emphasize that cameras are but one apparatus of the surveillance machine, and other forms of surveillance may be deployed disproportionately in Black neighborhoods. Furthermore, while our findings highlight that the prevalence and increase in surveillance cameras are related to increasing diversity as white residents move into non-white areas, we do not claim that surveillance is absent in other neighborhood change contexts, such as Black gentrification. Prior research has documented how Black gentrifiers can also surveil incumbent Black residents and these forms of surveillance may differ from the surveillance cameras we examine here.

One limitation of our study is the potential for endogeneity in these relationships. Specifically, although we suggest that cameras are installed in response to increasing diversity, alternative explanations may hold. It is possible, for example, that preexisting surveillance cameras influence patterns of racial change. We take several steps to mitigate this concern: our models control for prior camera prevalence to ensure that our estimates capture changes in surveillance cameras rather than simply reflecting areas that already had more cameras. In addition, we incorporate lagged measures of racial diversity and other neighborhood characteristics to check that demographic changes precede changes in camera prevalence rather than occurring simultaneously. Despite these steps, we acknowledge that surveillance and neighborhood racial composition may be mutually reinforcing over time.

A related consideration is the role of crime in shaping surveillance patterns. Our models include reported crime rates, but we emphasize that reported crime is not a neutral measure of criminal activity. Instead, reported crime reflects differential reporting practices across neighborhoods shaped by racial composition, perceptions of disorder and other social processes. Thus, in our context, crime should not be viewed as an exogenous predictor of surveillance but rather as part of the same social process that drives camera placement. Future research could explore quasi-experimental approaches to better isolate the dynamics between crime perceptions, crime reporting and camera placement.

Our findings suggest several other pathways for future research. First, we call on future research to examine the patterns we observe within cities’ political and historical contexts. With our dataset and approach, we cannot determine who owns cameras, whether they are functional or who is viewing the footage of the cameras. Thus, while we observe the distribution of surveillance infrastructure, we cannot directly observe the motivations behind their placement or how frequently cameras are used. The process of surveillance camera placement and increase is unknown to us but may be very revealing in the mechanisms underlying uneven surveillance; future research...
could benefit from data that distinguish between camera owners and functionality to better understand the relationship between surveillance cameras and neighborhood dynamics. Furthermore, given that we are using street-level imagery and that some of our years of analysis predate the preponderance of doorbell cameras, small cameras and indoor cameras are not detectable and, thus, are not included in our analysis. This exclusion raises the question of whether the prevalence of doorbell cameras in certain neighborhoods may relate to the installation of traditionally mounted surveillance cameras, and thus we may underestimate the true degree of surveillance in neighborhoods. As image quality improves, allowing better detection of small features, we call on future research to examine the usage of these smaller cameras as a means of social control enabled by our methodology. In particular, future research could clarify whether doorbell cameras reinforce or reshape existing patterns of camera placement. The role of policies in shaping the distribution of surveillance cameras also warrants further attention. Given the potential for surveillance cameras to reinforce spatial inequalities, policy interventions could include greater community involvement in decision-making of where cameras are placed or regulations managing the deployment of surveillance cameras to ensure they are not disproportionately placed in certain communities.

Scholars have forewarned that the widespread adoption of surveillance technologies may exacerbate racial inequality, but this was previously untestable at scale. Using ten large US cities, our findings indicate that the usage of surveillance cameras is, in fact, related to neighborhood racial demographics. Our findings suggest that cameras may be a neighborhood amenity for white residents to exert social control in racially heterogeneous settings. This may entrench racial inequality and erode social trust in diverse neighborhoods. As we have done here, we are hopeful that computational methods and visual data can intervene in other remaining questions about neighborhoods, social control and racialized spatial inequality.

Methods

We estimate the presence of visible surveillance cameras using a methodology that builds upon Sheng et al.15. Whereas their study aimed to detect cameras, we expand the approach by incorporating a longitudinal approach to assess changes in surveillance presence over time and integrating sociological theory to test how racial composition and demographic change predict surveillance camera prevalence and expansion. Our method identifies surveillance cameras mounted on buildings, street poles and other fixed structures in public spaces. It does not detect residential doorbell cameras, which are often smaller and not consistently visible in GSV imagery.

Here, we briefly describe the approach to detecting cameras in urban environments developed by Sheng et al.15. For a full technical account of the pipeline, we refer readers to that prior work. We note also that the methods described here builds upon the contribution of Turttiainen et al.50, who were among the first to suggest using computer vision algorithms and street view data to identify surveillance cameras.

We analyze data from the ten most densely populated cities with populations over 500,000 residents in the USA: Baltimore, Boston, Chicago, Los Angeles, Milwaukee, New York City, Philadelphia, San Francisco, Seattle and Washington, DC. We focus on the densest cities to examine surveillance camera patterns in areas where population concentration and the built environment create a higher likelihood of surveillance camera deployment. Imagery data come from the Google Static Streetview application programming interface for each city. We also source road network data from OpenStreetMap51,52. To develop a labeled dataset for training and evaluating our detection model, we obtained verified camera locations from the Electronic Frontier Foundation (EFF) in San Francisco53 and data from Mapillary Vistas, a global collection of street-level imagery with a small but diverse set of labeled surveillance cameras54.

For a set of confirmed surveillance cameras identified by EFF, we pull the closest GSV images. We supplement these data with camera instances from Mapillary Vistas. We partition the positive images into training (70%), validation (15%) and test (15%) sets split by location. To account for instances where surveillance cameras may look like similar urban features, we also include cases where cameras were listed in the EFF dataset but are not visible.

Next, we sample 100,000 points chosen uniformly from the road network in the year 2015. At each sampled point, we capture a 360° panorama and a single 90° field of view, oriented perpendicular to the road’s direction, to maximize visibility of nearby structures that could have surveillance cameras.

Finally, we run our camera detection model on the resulting set of 100,000 images in each of the ten cities. Our detection model uses the architecture of DeepLab V3+55,56 with an EfficientNet-b3 backbone57. Despite its accuracy, the model is subject to certain error patterns, including false positives (occasionally detecting objects with features visually similar to cameras, such as fixtures or street signs) and under-counting (when multiple cameras are clustered in a single image). To address these issues, we use human verification to review and confirm valid camera detections as the final step. We rely only on images with cameras verified by a human annotator.

Our model is also subject to false negatives, where actual cameras are not detected by the model. To assess the extent of this issue, we ran our trained model on the held-out validation dataset and estimated the model’s recall as 0.63. This overall recall value estimates that approximately 63% of actual surveillance cameras were detected; thus, our results should be interpreted recognizing that we may be slightly underestimating camera prevalence.

We undergo a similar process to compile a longitudinal dataset. However, for each uniformly sampled point along the road network, we now identify the earliest and the latest available image and limit each city to 50,000 earliest (median year of 2007) and 50,000 latest (median year of 2019) images. For an illustration of this pipeline, from the raw image to segmentation and bounding boxes to human verification, see fig. 7 in the work of Sheng et al.15. See Supplementary Fig. 1 for examples of verified camera detections and Supplementary Fig. 2 for detection maps, along with the population and road length, in our ten cities of analysis.

Throughout, we operationalize a ‘neighborhood’ as a census block group. Census block groups contain 600–2,000 residents on average and are nested within census tracts. Our volume of images enables measurement and analysis at this finer scale. For each block group, we calculate a static number of cameras (based on 2015) and whether cameras increased over the full analysis period (2007–2021). To estimate the cross-sectional relationship between camera count and neighborhood characteristics, we use zero-inflated Poisson regression given that we are modeling count data and our dependent variable, block-group level camera count, is overdispersed. We also expect excess zeros to be generated by a separate process from counts58. We include an offset term for the road length in kilometers within each block group and a weight for the image count. This choice reflects our data collection process: because our sampling is based on GSV images captured along the road network, neighborhoods with longer roads inherently yield more sampled images and, thus, more detected cameras. Using road length as an offset allows us to accurately estimate the density of surveillance cameras per unit of public space available for monitoring. To estimate how neighborhood change relates to the change in camera prevalence over time, we similarly fit zero-inflated Poisson models where the outcome is camera count in the latest year, controlling for the initial number of cameras. We rely on the 2015–2019 American Community Survey for all neighborhood-level characteristics for cross-sectional models. When estimating the relationship between camera gain and change in neighborhood characteristics over time, we use the 2006–2010 and 2015–2019 American Community...
Surveys, as these correspond to the median years of the earliest and latest images for each location in our dataset. In all multivariate models, we control for population, median home value, median household income, housing vacancy rate and the most prevalent zoning in each block group sourced from each city’s publicly available zoning data. However, results are substantively similar in models excluding them (as shown in Supplementary Table 3) and models where we control for population density instead of total population. Each of these controls is intended to account for socioeconomic and structural characteristics of a neighborhood that may relate to racial composition and camera prevalence, thus potentially confounding the key relationships of interest. Because the placement and location of surveillance cameras is a city-specific process, all multivariate models include city fixed effects. For the change models, we include the baseline levels and change in each control but assume that modal zoning remains constant. The zero component accounts for the city and road length in all models. We estimate all regression models in R.

Our focal characteristics are racial composition, diversity and crime. For racial composition, we focus on non-Hispanic Black and white (throughout, ‘Black’ and ‘white’, respectively). residents within a neighborhood. Although urban neighborhoods are composed of groups beyond these, this focus is driven by the theoretical perspectives that we intend to test; research on surveillance highlights the racial biases that disproportionately affect Black communities in urban settings, while white residents are frequently viewed as the dominant group within a neighborhood. However, we also test how camera prevalence relates to neighborhood diversity, which considers groups beyond Black and white residents. In particular, to measure diversity, we use the Shannon entropy measure

$$H = - \sum_{i=1}^{R} p_i \ln p_i,$$

where $p_i$ refers to the proportion of individuals in the spatial unit belonging to the $i$th group. The racial categories comprising $R$ are white, Black, Asian, Hispanic and other. The minimum value the measure can take is zero, where only one group is present; higher values indicate greater diversity. We use the Shannon measure instead of other diversity measures (for example, Simpson) because it captures both the richness and the evenness of racial composition, thus considering the relative abundance of each group.

Note that this measure is derived from the percentages that define racial composition; thus, to distinguish their effects, we residualize the measures of racial composition when we include them in a model with entropy by first regressing the percentages of the racial groups on entropy and then using the residuals in the model. Therefore, we capture the relationship between camera identification and the share of Black and white residents independent of these shares’ contribution to entropy. However, as shown in Supplementary Table 3, the results are similar to those of models that do not residualize these measures.

Throughout, we also consider how the presence of cameras and their increase relate to crime as surveillance cameras are purportedly deployed to respond to crime. We source crime data for several of our cities from the Crime Open Database. We use publicly available crime data for cities not available in the Crime Open Database and categorize all crimes according to the National Incident-Based Reporting System. We include all types of crime, including property, violent and other crimes against persons or society, and generate a per-capita total crime rate (to account for population differences across neighborhoods) at the block-group level. These crime rates may not reflect all crime in a neighborhood given that many crimes go unreported. However, we intentionally use the total crime rate because cameras are a response not just to the true level of crime in a neighborhood, but also to where crime is perceived as a problem. Thus, the total crime rate captures social processes that are of interest to us. As a robustness check, we also estimated a per-capita crime rate for violent crimes and motor vehicle thefts, which are more consistently reported. This measure is highly correlated with total reported crime, and our results remain nearly identical when using them (Supplementary Tables 4 and 7). Given this correlation and our theoretical interest in crime reporting as a social process, we proceed with the total per-capita crime rate as our primary measure.

**Reporting summary**

Further information on research design is available in the Nature Portfolio Reporting Summary linked to this article.

**Data availability**

All datasets used in the analysis for this study are publicly available via the Stanford Digital Repository at https://doi.org/10.25740/jr882ny4955 (ref. 61). Derived crime measures are included in our dataset, but original data on crime are available via the Crime Open Database at https://osf.io/zyaqn/. Google Street View imagery data cannot be made publicly available here due to copyright restrictions, but are accessible via Google.

**Code availability**

All code used in the analysis for this study is publicly available via GitHub at https://github.com/Changing-Cities-Research-Lab/surveillance-replication.

**References**

1. Brayne, S. Big data surveillance: the case of policing. *Am. Sociol. Rev.* **82**, 977–1008 (2017).
2. Lyon, D. *The Electronic Eye: The Rise of Surveillance Society* (Univ. Minnesota Press, 1994).
3. Brayne, S. The banality of surveillance. *Surveill. Soc.* **20**, 372–378 (2022).
4. Haggerty, K. D. & Ericson, R. V. (eds) *The New Politics of Surveillance and Visibility* (Univ. Toronto Press, 2019).
5. Bell, M. C. Anti-segregation policing. *NYUL Rev.* **95**, 650–765 (2020).
6. Lyon, D. *Identifying Citizens: ID Cards as Surveillance* (Polity, 2009).
7. Selod, S. *Forever Suspect: Racialized Surveillance of Muslim Americans in the War on Terror* (Rutgers Univ. Press, 2018).
8. Asad, A. L. *Engage and Evade: How Latino Immigrant Families Manage Surveillance in Everyday Life* (Princeton Univ. Press, 2023).
9. Armenta, A. *Protect, Serve, and Deport: The Rise of Policing as Immigration Enforcement* (Univ. California Press, 2017).
10. Ball, K. Workplace surveillance: an overview. *Labor Hist.* **51**, 87–106 (2010).
11. Egbert, S. & Leese, M. *Criminal Futures: Predictive Policing and Everyday Police Work* (Taylor & Francis, 2021).
12. Lageson, S. E. *Digital Punishment: Privacy, Stigma, and the Harms of Data-Driven Criminal Justice* (Oxford Univ. Press, 2020).
13. Fong, K. Getting eyes in the home: child protective services investigations and state surveillance of family life. *Am. Sociol. Rev.* **85**, 610–638 (2020).
14. Hughes, C. C. A house but not a home: how surveillance in subsidized housing exacerbates poverty and reinforces marginalization. *Soc. Forces* **100**, 293–315 (2021).
15. Sheng, H., Yao, K. & Goel, S. Surveilling surveillance: estimating the prevalence of surveillance cameras with street view data. *Proc. AAAI/ACM Conference on AI, Ethics, and Society (AIES ’21)* 221–230 (2021).
16. Goold, B., Loader, I. & Thumala, A. The banality of security: the curious case of surveillance cameras. *Br. J. Criminol.* **53**, 977–996 (2013).
17. Janowitz, M. Sociological theory and social control. *Am. J. Sociol.* **81**, 82–108 (1975).

18. La Vigne, N. G., Lowry, S. S., Markman, J. A. & Dwyer, A. M. *Evaluating the Use of Public Surveillance Cameras for Crime Control and Prevention Report NCJ236795* (Urban Institute, Washington, 2011).

19. Ratcliffe, J. H., Taniguchi, T. & Taylor, R. B. The crime reduction effects of public CCTV cameras: a multi-method spatial approach. *Justice Q.* **26**, 746–770 (2009).

20. Sampson, R. J. *Great American City: Chicago and the Enduring Neighborhood Effect* (Univ. Chicago Press, 2012).

21. Piza, E. L., Welsh, B. C., Farrington, D. P. & Thomas, A. L. CCTV surveillance for crime prevention. *Criminol. Public Policy* **18**, 135–159 (2019).

22. Wacquant, L. Deadly symbiosis: when ghetto and prison meet and mesh. *Punishm. Soc.* **3**, 95–133 (2001).

23. Shedd, C. Countering the carceral continuum: the legal of mass incarceration. *Criminal. Public Policy* **10**, 865 (2011).

24. Brydolf-Horwitz, M. & Beckett, K. In *Research in Political Sociology* (ed. Pettinicchio, D.) 91–111 (Emerald Publishing, 2021); https://doi.org/10.1108/S0895-9935202100000028005

25. Eitle, D., D’Alessio, S. J. & Stolzenberg, L. Racial threat and social control: a test of the political, economic, and threat of black crime hypotheses. *Soc. Forces* **81**, 557–576 (2002).

26. DeFina, R. & Hannon, L. Diversity, racial threat and metropolitan housing segregation. *Soc. Forces* **88**, 373–394 (2009).

27. Jacobs, D. & O’Brien, R. M. The determinants of deadly force: a structural analysis of police violence. *Am. J. Sociol.* **103**, 837–862 (1998).

28. Novak, K. J. & Chamlin, M. B. Racial threat, suspicion, and police behavior: the impact of race and place in traffic enforcement. *Crime Delinq.* **58**, 275–300 (2012).

29. Stults, B. J. & Baumer, E. P. Racial context and police force size: evaluating the empirical validity of the minority threat perspective. *Am. J. Sociol.* **113**, 507–546 (2007).

30. Blalock, H. M. Status inconsistency, social mobility, status integration and structural effects. *Am. Sociol. Rev.* **32**, 790–801 (1967).

31. Beck, B. Broken windows in the cul-de-sac? Race/ethnicity and quality-of-life policing in the changing suburbs. *Crime Delinq.* **65**, 270–292 (2019).

32. Bobo, L. & Hutchings, V. L. Perceptions of racial group competition: extending Blumer’s theory of group position to a multiracial social context. *Am. Sociol. Rev.* **61**, 951–973 (1996).

33. Kent, S. L. & Jacobs, D. Minority threat and police strength from 1980 to 2000: a fixed-effects analysis of nonlinear and interactive effects in large U.S. cities. *Criminology* **43**, 731–760 (2005).

34. Liska, A. E. *Social Threat and Social Control* (Suny Press, 1992).

35. Duxbury, S. W. & Andrabi, N. The boys in blue are watching you: the shifting metropolitan landscape and big data police surveillance in the United States. *Soc. Probbl.* **71**, 912–937 (2024).

36. Dinesen, P. T., Schaeffer, M. & Sønderskov, K. M. Ethnic diversity and social trust: a narrative and meta-analytical review. *Annu. Rev. Polit. Sci.* **23**, 441–465 (2020).

37. Perry, E. M. *Live and Let Live: Diversity, Conflict, and Community in an Integrated Neighborhood* (UNC Press Books, 2016).

38. Suttles, G. D. *The Social Construction of Communities* Vol. 111 (Univ. Chicago Press, 1972).

39. Kadowaki, J. The contemporary defended neighborhood: maintaining stability and diversity through processes of community defense. *City Commun.* **18**, 1220–1239 (2019).

40. Walton, E. Habits of whiteness: how racial domination persists in multiethnic neighborhoods. *Sociol. Race Ethn.* **7**, 71–85 (2021).

41. Freeman, L. *There Goes the Hood: Views of Gentrification from the Ground Up* (Temple Univ. Press, 2006).

42. Doering, J. *Us versus Them: Race, Crime, and Gentrification in Chicago Neighborhoods* (Oxford Univ. Press, 2020); https://doi.org/10.1093/oso/9780190066574.001.0001

43. Dahir, N., Hwang, J. & Yu, A. Cleaning up the neighborhood: white influx and differential requests for services. *Socius Sociol. Res. Dyn. World* **10**, 23780231231223436 (2024).

44. Douds, K. W. The diversity contract: constructing racial harmony in a diverse American suburb. *Am. J. Sociol.* **126**, 1347–1388 (2021).

45. Abascal, M. & Baldassarri, D. Love thy neighbor? Ethnoracial diversity and trust reexamined. *Am. J. Sociol.* **121**, 722–782 (2015).

46. Legewie, J. & Schaeffer, M. Contested boundaries: explaining where ethnoracial diversity provokes neighborhood conflict. *Am. J. Sociol.* **122**, 125–161 (2016).

47. Zukin, S. Gentrification: culture and capital in the urban core. *Annu. Rev. Sociol.* **13**, 129–147 (1987).

48. Brown-Saracino, J. *A Neighborhood That Never Changes: Gentrification, Social Preservation, and the Search for Authenticity* (Univ. Chicago Press, 2019).

49. Pattillo, M. *Black on the Block: The Politics of Race and Class in the City* (Univ. Chicago Press, 2010).

50. Turittainen, H., Costin, A., Lahtinen, T., Sintonen, L. & Hamalainen, T. Towards large-scale, automated, accurate detection of CCTV camera objects using computer vision. Applications and implications for privacy, safety, and cybersecurity. Preprint at https://doi.org/10.48550/arXiv.2006.03870 (2020).

51. Boeing, G. OSMnx: new methods for acquiring, constructing, analyzing, and visualizing complex street networks. *Comput. Environ. Urban Syst.* **65**, 126–139 (2017).

52. Planet Dump. OpenStreetMap https://planet.osm.org (2017).

53. Maass, D. The San Francisco District Attorney’s 10 most surveilled neighborhoods. Electronic Frontier Foundation https://www.eff.org/deeplinks/2019/02/san-francisco-district-attorneys-10-most-surveilled-places (2019).

54. Neuhold, G., Ollmann, T., Rota Bulo, S. & Kontschieder, P. The Mapillary Vista dataset for semantic understanding of street scenes. In *Proc. IEEE International Conference on Computer Vision* 4990–4999 (IEEE, 2017).

55. Chen, L.-C., Zhu, Y., Papandreou, G., Schroff, F. & Adam, H. Encoder–decoder with atrous separable convolution for semantic image segmentation. In *Proc. European Conference on Computer Vision* 801–818 (2018).

56. Chen, L.-C., Papandreou, G., Schroff, F. & Adam, H. Rethinking atrous convolution for semantic image segmentation. Preprint at https://arxiv.org/abs/1706.05587 (2017).

57. Tan, M. & Le, Q. Efficientnet: rethinking model scaling for convolutional neural networks. In *International Conference on Machine Learning* 6105–6114 (PMLR, 2019).

58. Lambert, D. Zero-inflated Poisson regression, with an application to defects in manufacturing. *Technometrics* **34**, 1–14 (1992).

59. Ashby, M. Crime Open Database (CODE). Open Science Framework https://doi.org/10.17605/OSF.IO/ZYAQN (2020).

60. Baumer, E. P. & Lauritsen, J. L. Reporting crime to the police, 1973–2005: a multivariate analysis of long-term trends in the National Crime Survey (NCS) and National Crime Victimization Survey (NCVS). *Criminology* **48**, 131–185 (2010).

61. Dahir, N., Sheng, H., Yao, K., Goel, S. & Hwang, J. Online appendix and data for Dahir et al. ‘Surveillance cameras are most prevalent in racially diverse neighborhoods across ten US cities’. Stanford Digital Repository https://doi.org/10.25740/jr882ny4955 (2025).

**Acknowledgements**

We thank S. Brayne, J. Doering, E. Eife and the participants and audience of the 2022 American Sociological Association panel on...
‘Surveillance and State Control’ for their helpful comments on earlier versions of this Article.

**Author contributions**
N.D., J.H. and S.G. conceptualized the study. N.D. and J.H. analyzed data. H.S., K.Y. and S.G. contributed to data curation and analytic tools. N.D. wrote the original draft, with review and editing by J.H. and S.G.

**Competing interests**
The authors declare no competing interests.

**Additional information**
**Supplementary information** The online version contains supplementary material available at https://doi.org/10.1038/s44284-025-00274-2.

**Correspondence and requests for materials** should be addressed to Nima Dahir.

**Peer review information** *Nature Cities* thanks Brenden Beck, Guangwen Song and the other, anonymous, reviewer(s) for their contribution to the peer review of this work.

**Reprints and permissions information** is available at www.nature.com/reprints.

**Publisher’s note** Springer Nature remains neutral with regard to jurisdictional claims in published maps and institutional affiliations.

Springer Nature or its licensor (e.g. a society or other partner) holds exclusive rights to this article under a publishing agreement with the author(s) or other rightsholder(s); author self-archiving of the accepted manuscript version of this article is solely governed by the terms of such publishing agreement and applicable law.

© The Author(s), under exclusive licence to Springer Nature America, Inc. 2025